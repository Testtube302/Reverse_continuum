//! Verifies every sound/*.wa2 decodes with the audio backend. Usage: sound_check <assets dir>
use macroquad::audio::load_sound_from_bytes;

#[macroquad::main("sound_check")]
async fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("assets dir")).join("sound");
    let (mut ok, mut bad) = (0, Vec::new());
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "wa2") {
            match load_sound_from_bytes(&std::fs::read(&p).unwrap()).await {
                Ok(_) => ok += 1,
                Err(err) => bad.push(format!("{}: {err:?}", p.display())),
            }
        }
    }
    println!("decoded {ok}, failed {}", bad.len());
    for b in bad.iter().take(5) {
        println!("  {b}");
    }
}
