//! Streaming / AI availability checks (解锁检测), made through the core's
//! mixed port so each service is reached the way the user's rules route it.
//! Each verdict is a pure function of the response, tested below.

use std::time::Duration;

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// Available.
    Yes,
    /// Partly (e.g. Netflix originals only).
    Partial,
    No,
    /// Couldn't tell (network error, challenge page).
    Unknown,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Check {
    pub service: &'static str,
    pub verdict: Verdict,
    /// Two-letter region when the service tells it.
    pub region: Option<String>,
    pub note: Option<String>,
}

/// What a check saw: final URL after redirects, status, body.
pub struct Seen {
    pub url: String,
    pub status: u16,
    pub body: String,
}

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

fn check(service: &'static str, verdict: Verdict, region: Option<String>, note: Option<&str>) -> Check {
    Check { service, verdict, region, note: note.map(String::from) }
}

/// Text between `start` and the next `end`.
fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let j = s[i..].find(end)?;
    Some(&s[i..i + j])
}

fn region_code(s: &str) -> Option<String> {
    let s = s.trim();
    (s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic())).then(|| s.to_ascii_uppercase())
}

/// Region from a URL path like `/hk/title/...` or `/jp-en/title/...`.
fn path_region(url: &str) -> Option<String> {
    let path = url.split("://").nth(1)?.split_once('/')?.1;
    region_code(path.split('/').next()?.split('-').next()?)
}

/// `title/81280792` is licensed (not a Netflix original); `title/80018499`
/// is an original.
pub fn netflix(licensed: &Seen, original: &Seen) -> Check {
    match (licensed.status, original.status) {
        (200, _) => check("Netflix", Verdict::Yes, path_region(&licensed.url).or(Some("US".into())), None),
        (404, 200) => check("Netflix", Verdict::Partial, path_region(&original.url), Some("仅自制剧")),
        (403, _) | (404, _) => check("Netflix", Verdict::No, None, None),
        (s, _) => check("Netflix", Verdict::Unknown, None, Some(&format!("HTTP {s}"))),
    }
}

pub fn youtube_premium(s: &Seen) -> Check {
    if s.body.contains("Premium is not available in your country") || s.body.contains("YouTube Premium 在您所在的国家/地区尚未推出") {
        return check("YouTube Premium", Verdict::No, None, None);
    }
    let region = between(&s.body, "\"INNERTUBE_CONTEXT_GL\":\"", "\"").or_else(|| between(&s.body, "\"countryCode\":\"", "\"")).and_then(region_code);
    match (s.status, region) {
        (200, Some(r)) if r == "CN" => check("YouTube Premium", Verdict::No, Some(r), None),
        (200, r @ Some(_)) => check("YouTube Premium", Verdict::Yes, r, None),
        (st, _) => check("YouTube Premium", Verdict::Unknown, None, Some(&format!("HTTP {st}"))),
    }
}

/// `trace`: chatgpt.com/cdn-cgi/trace; `compliance`: the iOS app's
/// cookie_requirements endpoint, which names unsupported countries.
pub fn chatgpt(trace: &Seen, compliance: &Seen) -> Check {
    let region = between(&format!("\n{}", trace.body), "\nloc=", "\n").and_then(region_code);
    if compliance.body.contains("unsupported_country") || matches!(region.as_deref(), Some("CN" | "HK" | "MO" | "RU" | "KP" | "IR")) {
        return check("ChatGPT", Verdict::No, region, None);
    }
    if trace.status != 200 {
        return check("ChatGPT", Verdict::Unknown, None, Some(&format!("HTTP {}", trace.status)));
    }
    check("ChatGPT", Verdict::Yes, region, None)
}

pub fn claude(s: &Seen) -> Check {
    if s.url.contains("app-unavailable-in-region") {
        return check("Claude", Verdict::No, None, None);
    }
    match s.status {
        200..=399 => check("Claude", Verdict::Yes, None, None),
        st => check("Claude", Verdict::Unknown, None, Some(&format!("HTTP {st}"))),
    }
}

pub fn gemini(s: &Seen) -> Check {
    if s.status != 200 {
        return check("Gemini", Verdict::Unknown, None, Some(&format!("HTTP {}", s.status)));
    }
    // The page carries this flag only where Gemini is offered.
    if s.body.contains("45631641,null,true") {
        check("Gemini", Verdict::Yes, None, None)
    } else {
        check("Gemini", Verdict::No, None, None)
    }
}

pub fn prime_video(s: &Seen) -> Check {
    if s.body.contains("isServiceRestricted") {
        return check("Prime Video", Verdict::No, None, None);
    }
    match between(&s.body, "\"currentTerritory\":\"", "\"").and_then(region_code) {
        Some(r) => check("Prime Video", Verdict::Yes, Some(r), None),
        None => check("Prime Video", Verdict::Unknown, None, Some(&format!("HTTP {}", s.status))),
    }
}

pub fn tiktok(s: &Seen) -> Check {
    match between(&s.body, "\"region\":\"", "\"").and_then(region_code) {
        Some(r) if r == "CN" => check("TikTok", Verdict::No, Some(r), None),
        Some(r) => check("TikTok", Verdict::Yes, Some(r), None),
        None => check("TikTok", Verdict::Unknown, None, Some(&format!("HTTP {}", s.status))),
    }
}

async fn fetch(http: &reqwest::Client, url: &str) -> Option<Seen> {
    let r = http.get(url).header("Accept-Language", "en-US,en;q=0.9").send().await.ok()?;
    let final_url = r.url().to_string();
    let status = r.status().as_u16();
    let body = r.text().await.unwrap_or_default();
    Some(Seen { url: final_url, status, body })
}

fn unreachable(service: &'static str) -> Check {
    check(service, Verdict::Unknown, None, Some("连接失败"))
}

/// Runs every check through the proxy on `port` (None: directly).
pub async fn run_all(port: Option<u16>) -> anyhow::Result<Vec<Check>> {
    let mut b = reqwest::Client::builder();
    b = match port {
        Some(p) => b.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{p}"))?),
        None => b.no_proxy(),
    };
    let http = b
        .user_agent(UA)
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()?;
    let h = &http;
    let netflix = async {
        let (a, b) = tokio::join!(fetch(h, "https://www.netflix.com/title/81280792"), fetch(h, "https://www.netflix.com/title/80018499"));
        match (a, b) {
            (Some(a), Some(b)) => netflix(&a, &b),
            _ => unreachable("Netflix"),
        }
    };
    let chatgpt = async {
        let (t, c) = tokio::join!(fetch(h, "https://chatgpt.com/cdn-cgi/trace"), fetch(h, "https://ios.chat.openai.com/public-api/compliance/cookie_requirements"));
        match (t, c) {
            (Some(t), c) => chatgpt(&t, &c.unwrap_or(Seen { url: String::new(), status: 0, body: String::new() })),
            _ => unreachable("ChatGPT"),
        }
    };
    let one = |service: &'static str, url: &'static str, f: fn(&Seen) -> Check| async move {
        match fetch(h, url).await {
            Some(s) => f(&s),
            None => unreachable(service),
        }
    };
    let (n, c, y, cl, g, p, t) = tokio::join!(
        netflix,
        chatgpt,
        one("YouTube Premium", "https://www.youtube.com/premium", youtube_premium),
        one("Claude", "https://claude.ai/", claude),
        one("Gemini", "https://gemini.google.com/", gemini),
        one("Prime Video", "https://www.primevideo.com/", prime_video),
        one("TikTok", "https://www.tiktok.com/explore", tiktok),
    );
    Ok(vec![n, y, p, t, c, cl, g])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opt-in, real network: PRISM_UNLOCK_LIVE=1 [PRISM_UNLOCK_PORT=7899]
    #[tokio::test]
    async fn live() {
        if std::env::var("PRISM_UNLOCK_LIVE").is_err() {
            return;
        }
        let port = std::env::var("PRISM_UNLOCK_PORT").ok().and_then(|p| p.parse().ok());
        for c in run_all(port).await.unwrap() {
            println!("{:16} {:?} {:?} {:?}", c.service, c.verdict, c.region, c.note);
        }
    }

    fn seen(url: &str, status: u16, body: &str) -> Seen {
        Seen { url: url.into(), status, body: body.into() }
    }

    #[test]
    fn verdicts() {
        let ok = seen("https://www.netflix.com/hk/title/81280792", 200, "");
        let orig = seen("https://www.netflix.com/hk/title/80018499", 200, "");
        assert_eq!(netflix(&ok, &orig), check("Netflix", Verdict::Yes, Some("HK".into()), None));
        let us = seen("https://www.netflix.com/title/81280792", 200, "");
        assert_eq!(netflix(&us, &orig).region.as_deref(), Some("US"));
        let jp = seen("https://www.netflix.com/jp-en/title/80018499", 200, "");
        let missing = seen("https://www.netflix.com/title/81280792", 404, "");
        assert_eq!(netflix(&missing, &jp), check("Netflix", Verdict::Partial, Some("JP".into()), Some("仅自制剧")));
        assert_eq!(netflix(&seen("", 403, ""), &seen("", 403, "")).verdict, Verdict::No);

        assert_eq!(youtube_premium(&seen("", 200, r#"x "INNERTUBE_CONTEXT_GL":"SG" y"#)).region.as_deref(), Some("SG"));
        assert_eq!(youtube_premium(&seen("", 200, "Premium is not available in your country")).verdict, Verdict::No);
        assert_eq!(youtube_premium(&seen("", 200, r#""countryCode":"CN""#)).verdict, Verdict::No);

        let trace = seen("", 200, "fl=1\nip=1.2.3.4\nloc=JP\ntls=TLSv1.3\n");
        assert_eq!(chatgpt(&trace, &seen("", 200, "{}")), check("ChatGPT", Verdict::Yes, Some("JP".into()), None));
        assert_eq!(chatgpt(&trace, &seen("", 403, r#"{"cf_details":"unsupported_country"}"#)).verdict, Verdict::No);
        assert_eq!(chatgpt(&seen("", 200, "loc=HK\n"), &seen("", 200, "")).verdict, Verdict::No);

        assert_eq!(claude(&seen("https://claude.ai/app-unavailable-in-region", 200, "")).verdict, Verdict::No);
        assert_eq!(claude(&seen("https://claude.ai/login", 200, "")).verdict, Verdict::Yes);
        assert_eq!(claude(&seen("https://claude.ai/", 403, "")).verdict, Verdict::Unknown);

        assert_eq!(gemini(&seen("", 200, "[45631641,null,true]")).verdict, Verdict::Yes);
        assert_eq!(gemini(&seen("", 200, "[45631641,null,false]")).verdict, Verdict::No);

        assert_eq!(prime_video(&seen("", 200, r#""currentTerritory":"US""#)).region.as_deref(), Some("US"));
        assert_eq!(prime_video(&seen("", 200, "isServiceRestricted")).verdict, Verdict::No);
        assert_eq!(tiktok(&seen("", 200, r#""region":"TW""#)).region.as_deref(), Some("TW"));
    }
}
