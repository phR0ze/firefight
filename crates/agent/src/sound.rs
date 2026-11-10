//! Sound playback on a dedicated thread (the audio stream is not `Send`).

use std::fs::File;
use std::sync::mpsc;
use std::time::Duration;

use firefight_core::model::TimerId;
use rodio::source::{SineWave, Zero};
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use tokio::sync::oneshot;

/// Names of the synthesized sounds every agent provides.
pub const BUNDLED: &[&str] = &["gong", "chime", "bell"];

type BoxedSource = Box<dyn Source + Send>;

enum Cmd {
    Play {
        timer_id: Option<TimerId>,
        sound: String,
        volume: f32,
        looping: bool,
        started: oneshot::Sender<Result<(), String>>,
        done: oneshot::Sender<()>,
    },
    Stop {
        timer_id: TimerId,
    },
}

#[derive(Clone)]
pub struct SoundPlayer {
    tx: mpsc::Sender<Cmd>,
}

impl SoundPlayer {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("firefight-sound".into())
            .spawn(move || sound_thread(rx))
            .expect("spawn sound thread");
        Self { tx }
    }

    /// Start a sound. Resolves once playback started (or failed); the returned
    /// receiver completes when the sound finishes or is stopped.
    pub async fn play(
        &self,
        timer_id: Option<TimerId>,
        sound: String,
        volume: f32,
        looping: bool,
    ) -> Result<oneshot::Receiver<()>, String> {
        let (started, started_rx) = oneshot::channel();
        let (done, done_rx) = oneshot::channel();
        self.tx
            .send(Cmd::Play { timer_id, sound, volume, looping, started, done })
            .map_err(|_| "sound thread stopped".to_string())?;
        started_rx.await.map_err(|_| "sound thread stopped".to_string())??;
        Ok(done_rx)
    }

    /// Stop every sound started for `timer_id`.
    pub fn stop(&self, timer_id: TimerId) {
        let _ = self.tx.send(Cmd::Stop { timer_id });
    }
}

struct Playing {
    timer_id: Option<TimerId>,
    sink: Sink,
    done: Option<oneshot::Sender<()>>,
}

fn sound_thread(rx: mpsc::Receiver<Cmd>) {
    let mut stream: Option<OutputStream> = None;
    let mut playing: Vec<Playing> = vec![];
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Cmd::Play { timer_id, sound, volume, looping, started, done }) => {
                let source = match load(&sound) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = started.send(Err(e));
                        continue;
                    }
                };
                if stream.is_none() {
                    match OutputStreamBuilder::open_default_stream() {
                        Ok(mut s) => {
                            s.log_on_drop(false);
                            stream = Some(s);
                        }
                        Err(e) => {
                            let _ = started.send(Err(format!("no audio output: {e}")));
                            continue;
                        }
                    }
                }
                let sink = Sink::connect_new(stream.as_ref().expect("opened").mixer());
                sink.set_volume(volume.clamp(0.0, 1.0));
                if looping {
                    sink.append(source.buffered().repeat_infinite());
                } else {
                    sink.append(source);
                }
                let _ = started.send(Ok(()));
                playing.push(Playing { timer_id, sink, done: Some(done) });
            }
            Ok(Cmd::Stop { timer_id }) => {
                for p in playing.iter().filter(|p| p.timer_id == Some(timer_id)) {
                    p.sink.stop();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        playing.retain_mut(|p| {
            if p.sink.empty() {
                if let Some(done) = p.done.take() {
                    let _ = done.send(());
                }
                false
            } else {
                true
            }
        });
    }
}

/// Resolve a bundled sound name or an absolute file path.
fn load(sound: &str) -> Result<BoxedSource, String> {
    if let Some(s) = synth(sound) {
        return Ok(s);
    }
    if !sound.starts_with('/') {
        return Err(format!("unknown sound '{sound}' (bundled: {})", BUNDLED.join(", ")));
    }
    let file = File::open(sound).map_err(|e| format!("{sound}: {e}"))?;
    let decoder = Decoder::try_from(file).map_err(|e| format!("{sound}: {e}"))?;
    Ok(Box::new(decoder))
}

/// A decaying tone, padded with silence to `period` so loops breathe.
fn tone(freq: f32, decay: f32, gain: f32) -> impl Source + Send {
    let d = Duration::from_secs_f32(decay);
    SineWave::new(freq).take_duration(d).fade_out(d).amplify(gain)
}

fn padded<S: Source + Send + 'static>(s: S, period: f32) -> BoxedSource {
    let silence = Zero::new(s.channels(), s.sample_rate()).take_duration(Duration::from_secs_f32(period));
    Box::new(s.mix(silence))
}

fn synth(name: &str) -> Option<BoxedSource> {
    Some(match name {
        "gong" => padded(tone(110.0, 3.0, 0.6).mix(tone(165.0, 2.5, 0.3)).mix(tone(220.0, 1.5, 0.15)), 3.5),
        "chime" => padded(tone(1046.5, 0.6, 0.4).mix(tone(784.0, 0.8, 0.4).delay(Duration::from_millis(300))), 1.6),
        "bell" => padded(tone(880.0, 1.2, 0.5).mix(tone(1760.0, 0.6, 0.15)), 1.5),
        _ => return None,
    })
}
