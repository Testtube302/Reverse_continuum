//! Frontend for the startup screen: loads the skin and lists from the install, feeds macroquad
//! input to `launcher::Launcher` and shows its framebuffer 1:1.

use std::time::Instant;

use aicontinuum::assets::AssetDir;
use aicontinuum::launcher::{self, Action, Canvas, Launcher, Profile, Zone};
use aicontinuum::skin::Skin;
use macroquad::prelude::*;

use crate::Args;

/// Our own zone, added to a fresh zone list so there is always somewhere to play.
const HOME_ZONE: (&str, &str, u16, &str) =
    ("Testtube Test Zone", "47.207.117.75", 5000, "Test zone for AIContinuum, a new open-source SubSpace client. Continuum players welcome.");

pub struct MenuScreen {
    launcher: Launcher,
    canvas: Canvas,
    texture: Texture2D,
    assets_root: std::path::PathBuf,
    wheel: f32,
    last_click: Option<(Instant, (i32, i32))>,
}

impl MenuScreen {
    pub fn load(assets: &AssetDir) -> Result<Self, String> {
        let (skin, spectate) = Skin::from_menures(&assets.read("menures.dll")?)?;
        let ships = assets.graphic("ships")?;
        let font = assets.graphic("menutext")?;
        let profile = assets
            .read("profile.dat")
            .map(|b| Profile::parse(&String::from_utf8_lossy(&b)))
            .unwrap_or_else(|_| Profile::defaults());
        let mut zones = assets.read("zone.dat").map(|b| launcher::parse_zone_dat(&String::from_utf8_lossy(&b))).unwrap_or_default();
        if !zones.iter().any(|z| z.name == HOME_ZONE.0) {
            let mut z = Zone::new(HOME_ZONE.0, HOME_ZONE.1, HOME_ZONE.2);
            z.description = HOME_ZONE.3.into();
            z.shown = true;
            zones.push(z);
        }
        let templates = aicontinuum::dialog::templates(&assets.read("menu040.dll")?);
        // Installed skins (skins/*.skn); "._" files are macOS resource-fork leftovers.
        let mut extra: Vec<(String, Skin)> = std::fs::read_dir(assets.path("skins"))
            .map(|dir| {
                dir.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let stem = name.strip_suffix(".skn").or_else(|| name.strip_suffix(".SKN"))?.to_string();
                        if stem.starts_with('.') {
                            return None;
                        }
                        let skin = Skin::from_skn(&std::fs::read(e.path()).ok()?).ok()?;
                        Some((stem, skin))
                    })
                    .collect()
            })
            .unwrap_or_default();
        extra.sort_by_key(|(n, _)| n.to_ascii_lowercase());
        let tips: Vec<String> = assets
            .read("Tips.dat")
            .map(|b| String::from_utf8_lossy(&b).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
            .unwrap_or_default();
        let mut launcher = Launcher::new(skin, extra, tips, spectate, ships, font, templates, profile, zones);
        launcher.select_skin_from_profile();
        let (w, h) = launcher.size();
        let canvas = Canvas::new(w, h);
        let texture = Texture2D::from_rgba8(w as u16, h as u16, &canvas.px);
        texture.set_filter(FilterMode::Nearest);
        Ok(Self { launcher, canvas, texture, assets_root: assets.path(""), wheel: 0.0, last_click: None })
    }

    fn save(&mut self) {
        if !self.launcher.dirty {
            return;
        }
        self.launcher.dirty = false;
        let _ = std::fs::write(self.assets_root.join("profile.dat"), self.launcher.profile.write());
        let _ = std::fs::write(self.assets_root.join("zone.dat"), launcher::write_zone_dat(&self.launcher.zones));
    }

    fn input(&mut self) -> launcher::Input {
        let (mx, my) = mouse_position();
        let mouse = (mx as i32, my as i32);
        self.wheel += mouse_wheel().1;
        let wheel = self.wheel.trunc() as i32;
        self.wheel -= wheel as f32;
        let left_pressed = is_mouse_button_pressed(MouseButton::Left);
        let mut double_click = false;
        if left_pressed {
            let now = Instant::now();
            double_click = self.last_click.is_some_and(|(t, (x, y))| {
                now.duration_since(t).as_millis() < 500 && (x - mouse.0).abs() < 4 && (y - mouse.1).abs() < 4
            });
            self.last_click = if double_click { None } else { Some((now, mouse)) };
        }
        let mut chars = Vec::new();
        let ctrl = is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl);
        while let Some(c) = get_char_pressed() {
            if !c.is_control() && !ctrl {
                chars.push(c);
            }
        }
        launcher::Input {
            mouse,
            left_pressed,
            right_down: is_mouse_button_down(MouseButton::Right),
            double_click,
            wheel,
            chars,
            enter: is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter),
            escape: is_key_pressed(KeyCode::Escape),
            backspace: is_key_pressed(KeyCode::Backspace),
            tab: is_key_pressed(KeyCode::Tab),
            ctrl_letter: if is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl) {
                [
                    (KeyCode::P, 'p'),
                    (KeyCode::M, 'm'),
                    (KeyCode::K, 'k'),
                    (KeyCode::Z, 'z'),
                    (KeyCode::T, 't'),
                    (KeyCode::O, 'o'),
                    (KeyCode::A, 'a'),
                    (KeyCode::X, 'x'),
                    (KeyCode::E, 'e'),
                ]
                .iter()
                .find(|(k, _)| is_key_pressed(*k))
                .map(|(_, c)| *c)
            } else {
                None
            },
            left: is_key_pressed(KeyCode::Left),
            right: is_key_pressed(KeyCode::Right),
            up: is_key_pressed(KeyCode::Up),
            down: is_key_pressed(KeyCode::Down),
            home: is_key_pressed(KeyCode::Home),
            end: is_key_pressed(KeyCode::End),
            alt_f4: (is_key_down(KeyCode::LeftAlt) || is_key_down(KeyCode::RightAlt)) && is_key_pressed(KeyCode::F4),
        }
    }

    /// Runs the screen until Play or Quit.
    pub async fn run(&mut self, args: &Args) -> Option<Action> {
        let (mut w, mut h) = self.launcher.size();
        request_new_screen_size(w as f32, h as f32);
        let mut frame = 0u32;
        loop {
            let mut input = self.input();
            if let Some(m) = args.mouse {
                input.mouse = m;
            }
            input.right_down |= args.right_held;
            if frame >= 30 && (frame - 30) % 15 == 0 {
                if let Some(&p) = args.clicks.get(((frame - 30) / 15) as usize) {
                    input.mouse = p;
                    input.left_pressed = true;
                }
            } else if let Some(&p) = args.clicks.get(((frame.saturating_sub(30)) / 15) as usize).filter(|_| frame >= 30) {
                input.mouse = p;
            }
            // Test aid: open a dialog once the first pings have had time to come back.
            if let (Some(which), 20) = (&args.open, frame) {
                self.launcher.open(which);
            }
            let mut action = self.launcher.update(&input);
            if args.autoplay && frame == 30 {
                action = self.launcher.press_play();
            }
            self.save();
            if let Some(Action::Url(url)) = &action {
                open_url(url);
                action = None;
            }
            if action.is_some() {
                return action;
            }
            // A different skin can change the window size.
            if self.launcher.size() != (w, h) {
                (w, h) = self.launcher.size();
                request_new_screen_size(w as f32, h as f32);
                self.canvas = Canvas::new(w, h);
                self.texture = Texture2D::from_rgba8(w as u16, h as u16, &self.canvas.px);
                self.texture.set_filter(FilterMode::Nearest);
            }
            self.launcher.draw(&mut self.canvas);
            self.texture.update_from_bytes(w as u32, h as u32, &self.canvas.px);
            clear_background(BLACK);
            draw_texture_ex(&self.texture, 0.0, 0.0, WHITE, DrawTextureParams { dest_size: Some(vec2(w as f32, h as f32)), ..Default::default() });
            frame += 1;
            if let Some((path, frames)) = &args.screenshot {
                if frame >= *frames {
                    // Save the framebuffer itself: exact pixels regardless of display scaling.
                    let saved = image::RgbaImage::from_raw(w as u32, h as u32, self.canvas.px.clone()).map(|i| i.save(path));
                    if !matches!(saved, Some(Ok(()))) {
                        eprintln!("could not save {}", path.display());
                    }
                    println!("saved {}", path.display());
                    return None;
                }
            }
            next_frame().await;
        }
    }
}

/// Opens a web page in the system browser.
fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = r {
        eprintln!("could not open {url}: {e}");
    }
}
