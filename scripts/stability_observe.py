"""Read-only, bounded telemetry collection. Never changes routes or restarts cores."""
import argparse
import ipaddress
import json
import math
import os
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

NUMBERS = ("process_id", "captured_at", "uptime_ms", "running_pumps", "rx_packets", "rx_bytes", "tx_packets", "tx_bytes",
           "ring_full_drops", "rx_queue_high_water", "tx_queue_high_water", "max_queue_wait_ms", "last_rx_ms", "last_tx_ms",
           "working_set_bytes", "private_bytes", "process_handles", "cpu_time_100ns",
           "network_recoveries", "network_recovery_failures", "last_network_recovery_ms")
STAGES = {"adapter_rx", "adapter_tx", "to_stack", "from_stack", "stack_runner", "tcp_accept", "udp_accept"}
REASONS = {"task_exited", "device_receive_error", "device_send_error", "stack_input_error", "stack_output_error", "stack_runner_error"}

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None

def endpoint(value):
    uri = urllib.parse.urlsplit(value)
    if uri.scheme != "http" or uri.username or uri.password or uri.query or uri.fragment or uri.path not in ("", "/"):
        raise ValueError("Use only a loopback HTTP controller origin, without credentials or paths")
    if not ipaddress.ip_address(uri.hostname).is_loopback or not uri.port:
        raise ValueError("A literal loopback IP and port are required")
    return value.rstrip("/") + "/prism/health"

def project(value):
    if not isinstance(value, dict) or value.get("schema") != 1:
        raise ValueError("Unsupported diagnostic schema")
    result = {k: value[k] for k in NUMBERS if isinstance(value.get(k), int) and not isinstance(value[k], bool) and 0 <= value[k] < 2**64}
    result["enabled"] = value.get("enabled") is True
    version = value.get("version", "")
    if isinstance(version, str) and len(version) <= 64 and all(c.isascii() and (c.isalnum() or c in ".-+") for c in version):
        result["version"] = version
    fault = value.get("fault")
    if isinstance(fault, dict) and fault.get("stage") in STAGES and fault.get("reason") in REASONS:
        result["fault"] = {"stage": fault["stage"], "reason": fault["reason"]}
        if isinstance(fault.get("os_error"), int) and -2**31 <= fault["os_error"] < 2**31:
            result["fault"]["os_error"] = fault["os_error"]
    return result

def observe(args):
    url = endpoint(args.controller)
    token = os.environ.get("PRISM_CONTROLLER_SECRET", "")
    if len(token) > 1024 or any(ord(c) < 32 for c in token):
        raise ValueError("Invalid controller secret")
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    request = urllib.request.Request(url, headers={"Authorization": "Bearer " + token} if token else {})
    deadline = time.monotonic() + args.seconds
    summary = {"samples": 0, "errors": 0, "pid_changes": 0, "fault_samples": 0}
    previous = None
    args.output.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive creation avoids overwriting old experiment results.
    with args.output.open("x", encoding="utf-8") as output:
        while time.monotonic() < deadline:
            row = {"observed_at": int(time.time())}
            try:
                with opener.open(request, timeout=3) as response:
                    raw = response.read(65537)
                    if len(raw) > 65536:
                        raise ValueError("Oversized health response")
                health = project(json.loads(raw))
                pid = health.get("process_id")
                if pid is not None and previous is not None and pid != previous:
                    summary["pid_changes"] += 1
                if pid is not None:
                    previous = pid
                summary["fault_samples"] += int("fault" in health)
                row["health"] = health
            except urllib.error.HTTPError as error:
                summary["errors"] += 1
                row["error"] = "HTTP_" + str(error.code)
            except (OSError, ValueError, urllib.error.URLError):
                summary["errors"] += 1
                row["error"] = "probe_unavailable"
            summary["samples"] += 1
            output.write(json.dumps(row, ensure_ascii=False) + "\n")
            output.flush()
            time.sleep(max(0, min(args.interval, deadline - time.monotonic())))
    print(json.dumps(summary))
    return summary

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--controller", default="http://127.0.0.1:9097")
    parser.add_argument("--seconds", type=float, default=60)
    parser.add_argument("--interval", type=float, default=5)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not math.isfinite(args.seconds) or not math.isfinite(args.interval) or args.seconds <= 0 or args.interval < 0.1:
        parser.error("seconds must be positive; interval must be at least 0.1")
    observe(args)

if __name__ == "__main__":
    main()
