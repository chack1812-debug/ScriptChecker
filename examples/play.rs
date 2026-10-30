use std::sync::{Arc, Mutex};

use scriptchecker::{
    audio::load_wav,
    output::start_output,
    transport::{Mode, Transport},
};

fn main() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| "使い方: cargo run --example play -- <full_dialogue.wav>".to_owned())?;
    let audio = load_wav(std::path::Path::new(&path))?;
    let transport = Arc::new(Mutex::new(Transport::new(
        audio.frame_count(),
        audio.sample_rate,
    )));
    transport
        .lock()
        .map_err(|_| "Transport ロックを取得できません".to_owned())?
        .set_mode(Mode::Play);
    let _output = start_output(audio, transport)?;
    std::thread::sleep(std::time::Duration::from_secs(10));
    Ok(())
}
