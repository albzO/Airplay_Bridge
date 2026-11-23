//! 在独立线程执行桌面作业，以便 WASAPI 自行初始化 STA。
//! Run desktop jobs on fresh threads so WASAPI can initialize STA.
use airplay_desktop::DesktopJob;
use async_channel::{Receiver, bounded};
use std::thread;

pub fn dispatch<T: Send + 'static>(
    job: DesktopJob<T>,
) -> Result<Receiver<Result<T, String>>, String> {
    let (send, receive) = bounded(1);
    thread::Builder::new()
        .name("airplay-desktop-job".into())
        .spawn(move || {
            let _ = send.try_send(job());
        })
        .map_err(|error| format!("无法启动桌面任务：{error}"))?;
    Ok(receive)
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/gpui_jobs.rs"]
mod tests;
