use crate::hub::Hub;
use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
    sync::Arc,
};
use tokio::{io::unix::AsyncFd, sync::mpsc};
use tokio_util::sync::CancellationToken;

/// The caller owns a duplicate of the VpnService packet fd; never close Java's copy.
pub fn attach(
    fd: OwnedFd,
    hub: Arc<Hub>,
    cancel: CancellationToken,
) -> anyhow::Result<impl std::future::Future<Output = anyhow::Result<()>> + Send> {
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error().into());
    }
    let device = Arc::new(AsyncFd::new(fd)?);
    let (input, from_device) = mpsc::channel(256);
    let (to_device, mut output) = mpsc::channel::<Vec<u8>>(256);
    super::stack::run(&hub.core().general.tun, hub.clone(), from_device, to_device)?;
    let reader = device.clone();
    Ok(async move {
        let read = async move {
            let mut bytes = vec![0u8; 65536];
            loop {
                let mut ready = reader.readable().await?;
                let result = ready.try_io(|inner| {
                    let n = unsafe {
                        libc::read(
                            inner.get_ref().as_raw_fd(),
                            bytes.as_mut_ptr().cast(),
                            bytes.len(),
                        )
                    };
                    if n < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(n as usize)
                    }
                });
                match result {
                    Err(_) => continue,
                    Ok(Err(e)) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Ok(Err(e)) => return Err::<(), anyhow::Error>(e.into()),
                    Ok(Ok(0)) => anyhow::bail!("Android TUN descriptor closed"),
                    Ok(Ok(n)) => input
                        .send(bytes[..n].to_vec())
                        .await
                        .map_err(|_| anyhow::anyhow!("Android packet stack stopped"))?,
                }
            }
        };
        let write = async move {
            while let Some(packet) = output.recv().await {
                loop {
                    let mut ready = device.writable().await?;
                    match ready.try_io(|inner| {
                        let n = unsafe {
                            libc::write(
                                inner.get_ref().as_raw_fd(),
                                packet.as_ptr().cast(),
                                packet.len(),
                            )
                        };
                        if n < 0 {
                            Err(io::Error::last_os_error())
                        } else {
                            Ok(n as usize)
                        }
                    }) {
                        Err(_) => continue,
                        Ok(Err(e)) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Ok(Err(e)) => return Err::<(), anyhow::Error>(e.into()),
                        Ok(Ok(n)) if n == packet.len() => break,
                        _ => anyhow::bail!("short Android TUN packet write"),
                    }
                }
            }
            anyhow::bail!("Android packet stack stopped")
        };
        tokio::select! { _ = cancel.cancelled() => Ok(()), r = read => r, r = write => r }
    })
}
