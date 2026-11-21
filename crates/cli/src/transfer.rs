use indicatif::{ProgressBar, ProgressStyle};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;
pub use wyrmyon_transfer::{Pipe, await_ack, send_ack};
use wyrmyon_transport_classic::RecordPipe;
use wyrmyon_transport_iroh::{IrohPipe, Route, Traffic};

use crate::wyrm::{Look, Paint, Wyrm};

const FINALE: std::time::Duration = std::time::Duration::from_millis(800);

pub fn follow_pipe(pipe: &Pipe, bar: &ProgressBar) {
    if let Pipe::Iroh(pipe) = pipe {
        follow_route(pipe, bar);
    }
}

pub fn went_by(pipe: &Pipe) -> Option<String> {
    went(pipe.traffic()?)
}

pub fn follow_route(pipe: &IrohPipe, bar: &ProgressBar) {
    announce_routes(pipe.route(), pipe.first_route(), bar.clone());
}

pub fn follow_sent(pipe: &IrohPipe, bar: &ProgressBar) {
    let mut sent = pipe.sent();
    let bar = bar.clone();
    tokio::spawn(async move {
        while sent.changed().await.is_ok() {
            let n = *sent.borrow_and_update();
            bar.set_position(bar.length().map_or(n, |total| n.min(total)));
        }
    });
}

fn announce_routes(
    mut route: watch::Receiver<Option<Route>>,
    mut last: Option<Route>,
    bar: ProgressBar,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while route.changed().await.is_ok() {
            let now = *route.borrow_and_update();
            if let Some(said) = route_change(last, now) {
                bar.suspend(|| eprintln!("{said}"));
            }
            last = now.or(last);
        }
    })
}

fn route_change(before: Option<Route>, now: Option<Route>) -> Option<&'static str> {
    match (before?, now?) {
        (Route::Relay, Route::Direct) => Some("Now direct: hole-punching opened a path"),
        (Route::Direct, Route::Relay) => Some("Back through the relay: the direct path closed"),
        _ => None,
    }
}

fn went(traffic: Traffic) -> Option<String> {
    let Traffic { direct, relay } = traffic;
    match (direct, relay) {
        (0, 0) => None,
        (_, 0) => Some("Went direct".to_owned()),
        (0, _) => Some("Went through the relay".to_owned()),
        _ => {
            let total = u128::from(direct) + u128::from(relay);
            let share = (u128::from(direct) * 100 + total / 2) / total;
            Some(format!(
                "Went {}% direct, the rest through the relay",
                share.clamp(1, 99)
            ))
        }
    }
}

pub fn progress(size: u64, hidden: bool, ascii: bool, wormhole: bool) -> ProgressBar {
    if hidden {
        return ProgressBar::hidden();
    }
    let bar = ProgressBar::new(size);
    let look = Look {
        ascii: ascii || old_console(),
        paint: Paint::detect(),
        wormhole,
    };
    bar.set_style(
        ProgressStyle::with_template(
            "{wyrm} {bytes}/{total_bytes} {binary_bytes_per_sec} eta {eta}",
        )
        .expect("the progress template is valid")
        .with_key("wyrm", Wyrm::new(look)),
    );
    bar.enable_steady_tick(std::time::Duration::from_millis(50));
    bar
}

fn old_console() -> bool {
    cfg!(windows) && std::env::var_os("WT_SESSION").is_none()
}

pub fn spinner(hidden: bool, ascii: bool) -> ProgressBar {
    if hidden {
        return ProgressBar::hidden();
    }
    let bar = ProgressBar::new_spinner();
    let style = ProgressStyle::with_template("{spinner} {msg} ({elapsed})")
        .expect("the spinner template is valid");
    bar.set_style(if ascii || old_console() {
        style.tick_chars("-\\|/ ")
    } else {
        style
    });
    bar.enable_steady_tick(std::time::Duration::from_millis(100));
    bar
}

pub struct Cleared(pub ProgressBar);

impl Drop for Cleared {
    fn drop(&mut self) {
        self.0.finish_and_clear();
    }
}

pub async fn finale(bar: &ProgressBar) {
    if !bar.is_hidden() && !bar.is_finished() {
        tokio::time::sleep(FINALE).await;
    }
    bar.finish_and_clear();
}

pub async fn send_stream(
    pipe: &mut RecordPipe,
    source: &mut (dyn AsyncRead + Unpin + Send),
    size: u64,
    bar: &ProgressBar,
) -> anyhow::Result<[u8; 32]> {
    let digest =
        wyrmyon_transfer::send_stream(pipe, source, size, &|n| bar.set_position(n)).await?;
    finale(bar).await;
    Ok(digest)
}

pub async fn receive_stream(
    pipe: &mut RecordPipe,
    sink: &mut (dyn AsyncWrite + Unpin + Send),
    size: u64,
    bar: &ProgressBar,
) -> anyhow::Result<[u8; 32]> {
    let digest =
        wyrmyon_transfer::receive_stream(pipe, sink, size, &|n| bar.set_position(n)).await?;
    finale(bar).await;
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_route_summary_says_where_the_bytes_went() {
        let went = |direct, relay| went(Traffic { direct, relay });
        assert_eq!(went(0, 0), None);
        assert_eq!(went(5, 0).as_deref(), Some("Went direct"));
        assert_eq!(went(0, 5).as_deref(), Some("Went through the relay"));
        assert_eq!(
            went(97, 3).as_deref(),
            Some("Went 97% direct, the rest through the relay")
        );
        assert_eq!(
            went(u64::MAX, u64::MAX).as_deref(),
            Some("Went 50% direct, the rest through the relay")
        );
        let change = |before, now| route_change(before, now).unwrap_or_default();
        assert!(change(Some(Route::Relay), Some(Route::Direct)).starts_with("Now direct"));
        assert!(change(Some(Route::Direct), Some(Route::Relay)).starts_with("Back through"));
        for (before, now) in [
            (None, Some(Route::Direct)),
            (None, Some(Route::Relay)),
            (Some(Route::Relay), None),
            (Some(Route::Direct), Some(Route::Direct)),
        ] {
            assert_eq!(route_change(before, now), None, "{before:?} {now:?}");
        }
        assert_eq!(
            went(1, 1000).as_deref(),
            Some("Went 1% direct, the rest through the relay")
        );
        assert_eq!(
            went(999_999, 1).as_deref(),
            Some("Went 99% direct, the rest through the relay")
        );
        assert_eq!(
            went(2, 1).as_deref(),
            Some("Went 67% direct, the rest through the relay")
        );
    }

    #[tokio::test]
    async fn the_spinner_hides_on_request_and_ticks_in_ascii_when_asked() {
        assert!(spinner(true, false).is_hidden());
        let ascii = spinner(false, true);
        assert!(!ascii.is_finished());
        ascii.finish_and_clear();
    }

    #[tokio::test]
    async fn the_announcer_follows_the_route_until_the_pipe_goes() {
        let (tx, rx) = watch::channel(Some(Route::Relay));
        let announcer = announce_routes(rx, Some(Route::Relay), progress(0, true, false, false));
        for now in [Some(Route::Direct), None, Some(Route::Relay)] {
            tx.send(now).unwrap();
            tokio::task::yield_now().await;
        }
        drop(tx);
        announcer.await.unwrap();
    }
}
