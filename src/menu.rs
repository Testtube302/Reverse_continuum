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
        let launcher = Launcher::new(skin, spectate, ships, font, profile, zones);
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
        while let Some(c) = get_char_pressed() {
            if !c.is_control() {
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
        }
    }

    /// Runs the screen until Play or Quit.
    pub async fn run(&mut self, args: &Args) -> Option<Action> {
        let (w, h) = self.launcher.size();
        request_new_screen_size(w as f32, h as f32);
        if let Some(which) = &args.open {
            self.launcher.open(which);
        }
        let mut frame = 0u32;
        loop {
            let mut input = self.input();
            if let Some(m) = args.mouse {
                input.mouse = m;
            }
            input.right_down |= args.right_held;
            let mut action = self.launcher.update(&input);
            if args.autoplay && frame == 30 {
                action = self.launcher.press_play();
            }
            self.save();
            if action.is_some() {
                return action;
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
