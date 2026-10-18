//! Key definitions (`keyboard.dat`) and macros (`macro.dat`) in the install's own formats.
//!
//! A binding is key names separated by spaces, modifiers first: `Shift Tab`, `F6`, `Shift Ctrl`.
//! A binding is active while all its keys are down, unless another active binding uses a
//! strict superset of its keys: holding Shift+Tab lays a mine without also afterburning (Shift)
//! and Shift+Ctrl repels without also firing (Ctrl), as in the original.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Shift,
    Ctrl,
    Alt,
    Left,
    Right,
    Up,
    Down,
    Tab,
    Space,
    Enter,
    Backspace,
    Insert,
    Delete,
    Home,
    End,
    PgUp,
    PgDown,
    F(u8),
    Char(char),
}

impl Key {
    pub fn is_modifier(self) -> bool {
        matches!(self, Key::Shift | Key::Ctrl | Key::Alt)
    }

    pub fn name(self) -> String {
        match self {
            Key::Shift => "Shift".into(),
            Key::Ctrl => "Ctrl".into(),
            Key::Alt => "Alt".into(),
            Key::Left => "Left".into(),
            Key::Right => "Right".into(),
            Key::Up => "Up".into(),
            Key::Down => "Down".into(),
            Key::Tab => "Tab".into(),
            Key::Space => "Space".into(),
            Key::Enter => "Enter".into(),
            Key::Backspace => "Backspace".into(),
            Key::Insert => "Insert".into(),
            Key::Delete => "Delete".into(),
            Key::Home => "Home".into(),
            Key::End => "End".into(),
            Key::PgUp => "PgUp".into(),
            Key::PgDown => "PgDown".into(),
            Key::F(n) => format!("F{n}"),
            Key::Char(c) => c.to_ascii_uppercase().to_string(),
        }
    }

    pub fn parse(s: &str) -> Option<Key> {
        Some(match s {
            "Shift" => Key::Shift,
            "Ctrl" => Key::Ctrl,
            "Alt" => Key::Alt,
            "Left" => Key::Left,
            "Right" => Key::Right,
            "Up" => Key::Up,
            "Down" => Key::Down,
            "Tab" => Key::Tab,
            "Space" => Key::Space,
            "Enter" => Key::Enter,
            "Backspace" => Key::Backspace,
            "Insert" => Key::Insert,
            "Delete" => Key::Delete,
            "Home" => Key::Home,
            "End" => Key::End,
            "PgUp" => Key::PgUp,
            "PgDown" => Key::PgDown,
            f if f.len() >= 2 && f.starts_with('F') && f[1..].parse::<u8>().is_ok() => Key::F(f[1..].parse().ok()?),
            c if c.chars().count() == 1 => Key::Char(c.chars().next()?.to_ascii_uppercase()),
            _ => return None,
        })
    }
}

/// Keys of one binding; empty means unbound.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Binding(pub Vec<Key>);

impl Binding {
    pub fn parse(s: &str) -> Self {
        Self(s.split_whitespace().filter_map(Key::parse).collect())
    }

    pub fn name(&self) -> String {
        self.0.iter().map(|k| k.name()).collect::<Vec<_>>().join(" ")
    }

    /// Modifiers first (Shift, Ctrl, Alt), then the main key, as the original writes them.
    pub fn from_keys(mut keys: Vec<Key>) -> Self {
        let order = |k: &Key| match k {
            Key::Shift => 0,
            Key::Ctrl => 1,
            Key::Alt => 2,
            _ => 3,
        };
        keys.sort_by_key(order);
        keys.dedup();
        Self(keys)
    }

    fn down(&self, is_down: &impl Fn(Key) -> bool) -> bool {
        !self.0.is_empty() && self.0.iter().all(|k| is_down(*k))
    }

    fn within(&self, other: &Binding) -> bool {
        self.0.len() < other.0.len() && self.0.iter().all(|k| other.0.contains(k))
    }
}

/// Every action in keyboard.dat, in file order, with the Key Definitions dialog's button id.
pub const ACTIONS: [(&str, i32); 50] = [
    ("TurnLeft", 1037),
    ("TurnRight", 1053),
    ("Forward", 1054),
    ("Reverse", 1055),
    ("Afterburners", 1056),
    ("Bomb", 1066),
    ("Bullet", 1040),
    ("Mine", 1041),
    ("Thor", 1042),
    ("Multi", 1036),
    ("AntiWarp", 1049),
    ("Stealth", 1050),
    ("Cloak", 1051),
    ("XRadar", 1052),
    ("Repel", 1034),
    ("Burst", 1043),
    ("Warp", 1044),
    ("Portal", 1045),
    ("Decoy", 1035),
    ("Rocket", 1046),
    ("Brick", 1047),
    ("Attach", 1048),
    ("Cycle", 1038),
    ("PrevPlayer", 1057),
    ("NextPlayer", 1058),
    ("PrevPage", 1088),
    ("NextPage", 1089),
    ("PlaySpec", 1062),
    ("Message", 1063),
    ("ViewMap", 1087),
    ("Macro1", 1039),
    ("Macro2", 1068),
    ("Macro3", 1069),
    ("Macro4", 1070),
    ("Macro5", 1071),
    ("Macro6", 1072),
    ("Macro7", 1073),
    ("Macro8", 1074),
    ("Macro9", 1075),
    ("Macro10", 1076),
    ("Macro11", 1077),
    ("Macro12", 1078),
    ("Macro13", 1079),
    ("Macro14", 1080),
    ("Macro15", 1081),
    ("Macro16", 1082),
    ("Macro17", 1083),
    ("Macro18", 1084),
    ("Macro19", 1085),
    ("Macro20", 1086),
];

/// The install's default definitions.
const DEFAULTS: [(&str, &str); 50] = [
    ("TurnLeft", "Left"),
    ("TurnRight", "Right"),
    ("Forward", "Up"),
    ("Reverse", "Down"),
    ("Afterburners", "Shift"),
    ("Bomb", "Tab"),
    ("Bullet", "Ctrl"),
    ("Mine", "Shift Tab"),
    ("Thor", "F6"),
    ("Multi", "Delete"),
    ("AntiWarp", "Shift End"),
    ("Stealth", "Home"),
    ("Cloak", "Shift Home"),
    ("XRadar", "End"),
    ("Repel", "Shift Ctrl"),
    ("Burst", "Shift Delete"),
    ("Warp", "Insert"),
    ("Portal", "Shift Insert"),
    ("Decoy", "F5"),
    ("Rocket", "F3"),
    ("Brick", "F4"),
    ("Attach", "F7"),
    ("Cycle", "F2"),
    ("PrevPlayer", "PgUp"),
    ("NextPlayer", "PgDown"),
    ("PrevPage", "Shift PgUp"),
    ("NextPage", "Shift PgDown"),
    ("PlaySpec", "F11"),
    ("Message", ""),
    ("ViewMap", "Alt"),
    ("Macro1", "Shift F1"),
    ("Macro2", "Shift F2"),
    ("Macro3", "Shift F3"),
    ("Macro4", "Shift F4"),
    ("Macro5", "Shift F5"),
    ("Macro6", "Shift F6"),
    ("Macro7", "Shift F7"),
    ("Macro8", "Shift F8"),
    ("Macro9", ""),
    ("Macro10", ""),
    ("Macro11", ""),
    ("Macro12", ""),
    ("Macro13", ""),
    ("Macro14", ""),
    ("Macro15", ""),
    ("Macro16", ""),
    ("Macro17", "Shift F11"),
    ("Macro18", "Shift F12"),
    ("Macro19", ""),
    ("Macro20", ""),
];

#[derive(Clone, Debug, PartialEq)]
pub struct KeySet {
    pub name: String,
    pub req_msg_key: bool,
    pub joystick: bool,
    pub bindings: Vec<(String, Binding)>,
}

impl KeySet {
    pub fn defaults(name: &str) -> Self {
        Self {
            name: name.into(),
            req_msg_key: false,
            joystick: false,
            bindings: DEFAULTS.iter().map(|(a, b)| (a.to_string(), Binding::parse(b))).collect(),
        }
    }

    pub fn get(&self, action: &str) -> Binding {
        self.bindings.iter().find(|(a, _)| a == action).map(|(_, b)| b.clone()).unwrap_or_default()
    }

    pub fn set(&mut self, action: &str, b: Binding) {
        match self.bindings.iter_mut().find(|(a, _)| a == action) {
            Some(e) => e.1 = b,
            None => self.bindings.push((action.into(), b)),
        }
    }

    /// Runtime-only alternative; never replace a saved binding or steal another action's chord.
    pub fn add_alternate(&mut self, action: &str, binding: Binding) {
        if !binding.0.is_empty() && !self.bindings.iter().any(|(_, b)| *b == binding) {
            self.bindings.push((action.into(), binding));
        }
    }

    /// Actions whose bindings are held now, minus those covered by a larger held binding.
    pub fn active(&self, is_down: impl Fn(Key) -> bool) -> Vec<&str> {
        let held: Vec<&(String, Binding)> = self.bindings.iter().filter(|(_, b)| b.down(&is_down)).collect();
        let mut active = Vec::new();
        for (action, _) in held.iter().filter(|(_, b)| !held.iter().any(|(_, o)| b.within(o))) {
            if !active.contains(&action.as_str()) {
                active.push(action.as_str());
            }
        }
        active
    }

    /// Actions sharing a binding with another action.
    pub fn conflicts(&self) -> Vec<&str> {
        self.bindings
            .iter()
            .filter(|(a, b)| !b.0.is_empty() && self.bindings.iter().any(|(o, ob)| o != a && Binding::from_keys(ob.0.clone()) == Binding::from_keys(b.0.clone())))
            .map(|(a, _)| a.as_str())
            .collect()
    }
}

/// `KeyDef = Name` starts each set (the original writes one).
pub fn parse_keysets(text: &str) -> Vec<KeySet> {
    let mut out: Vec<KeySet> = Vec::new();
    for line in text.lines().filter(|l| !l.trim_start().starts_with(';')) {
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        if k.eq_ignore_ascii_case("KeyDef") {
            let set = KeySet::defaults(v);
            out.push(set);
            continue;
        }
        if out.is_empty() {
            out.push(KeySet::defaults("Default"));
        }
        let set = out.last_mut().unwrap();
        match k {
            "ReqMsgKey" => set.req_msg_key = v == "1",
            "Joystick" => set.joystick = v == "1",
            _ => set.set(k, Binding::parse(v)),
        }
    }
    if out.is_empty() {
        out.push(KeySet::defaults("Default"));
    }
    out
}

pub fn write_keysets(sets: &[KeySet]) -> String {
    let mut out = String::from("; This file is auto-generated. DO NOT EDIT.\r\n\r\n");
    for s in sets {
        out += &format!("{:<13}= {}\r\n", "KeyDef", s.name);
        out += &format!("{:<13}= {}\r\n", "ReqMsgKey", s.req_msg_key as u8);
        out += &format!("{:<13}= {}\r\n", "Joystick", s.joystick as u8);
        for (a, b) in &s.bindings {
            out += &format!("{a:<13}= {}\r\n", b.name());
        }
        out += "\r\n";
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct MacroSet {
    pub name: String,
    pub lines: [String; 20],
}

impl MacroSet {
    pub fn empty(name: &str) -> Self {
        Self { name: name.into(), lines: Default::default() }
    }
}

/// `macro = Name` starts each set, then `1 = text` .. `20 = text`.
pub fn parse_macros(text: &str) -> Vec<MacroSet> {
    let mut out: Vec<MacroSet> = Vec::new();
    for line in text.lines().filter(|l| !l.trim_start().starts_with(';')) {
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim_start().trim_end_matches(['\r', '\n']));
        if k.eq_ignore_ascii_case("macro") {
            out.push(MacroSet::empty(v.trim()));
            continue;
        }
        if out.is_empty() {
            out.push(MacroSet::empty("Default"));
        }
        if let Some(n) = k.parse::<usize>().ok().filter(|n| (1..=20).contains(n)) {
            out.last_mut().unwrap().lines[n - 1] = v.to_string();
        }
    }
    if out.is_empty() {
        out.push(MacroSet::empty("Default"));
    }
    out
}

pub fn write_macros(sets: &[MacroSet]) -> String {
    let mut out = String::from("; This file is auto-generated. DO NOT EDIT.\r\n\r\n");
    for s in sets {
        out += &format!("macro = {}\r\n", s.name);
        for (i, l) in s.lines.iter().enumerate() {
            out += &format!("{:<6}= {l}\r\n", i + 1);
        }
        out += "\r\n";
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternate_player_list_binding_preserves_f2_and_suppresses_map_modifier() {
        let mut set = KeySet::defaults("Default");
        set.add_alternate("Cycle", Binding::parse("Alt P"));
        assert_eq!(set.get("Cycle").name(), "F2");
        assert_eq!(set.active(|k| [Key::Alt, Key::Char('P')].contains(&k)), ["Cycle"]);
        assert_eq!(set.active(|k| k == Key::F(2)), ["Cycle"]);
        assert_eq!(set.active(|k| [Key::F(2), Key::Alt, Key::Char('P')].contains(&k)), ["Cycle"]);
        assert_eq!(set.active(|k| k == Key::Alt), ["ViewMap"]);
        let count = set.bindings.len();
        set.add_alternate("Cycle", Binding::parse("Alt P"));
        assert_eq!(set.bindings.len(), count);
        let mut custom = KeySet::defaults("Custom");
        custom.set("Portal", Binding::parse("Alt P"));
        custom.add_alternate("Cycle", Binding::parse("Alt P"));
        assert_eq!(custom.active(|k| [Key::Alt, Key::Char('P')].contains(&k)), ["Portal"]);
    }

    #[test]
    fn larger_bindings_win() {
        let set = KeySet::defaults("Default");
        let held = |keys: &'static [Key]| move |k: Key| keys.contains(&k);
        assert_eq!(set.active(held(&[Key::Shift])), ["Afterburners"]);
        assert_eq!(set.active(held(&[Key::Shift, Key::Tab])), ["Mine"]);
        assert_eq!(set.active(held(&[Key::Shift, Key::Ctrl])), ["Repel"]);
        assert_eq!(set.active(held(&[Key::Ctrl, Key::Up])), ["Forward", "Bullet"]);
        assert_eq!(set.active(held(&[Key::Shift, Key::F(6)])), ["Macro6"]);
        assert!(set.conflicts().is_empty());
    }

    #[test]
    fn files_round_trip() {
        let text = "; c\r\nKeyDef       = Default\r\nReqMsgKey    = 0\r\nBomb         = B\r\nMacro9       = Ctrl F9\r\n";
        let sets = parse_keysets(text);
        assert_eq!(sets[0].get("Bomb"), Binding(vec![Key::Char('B')]));
        assert_eq!(sets[0].get("Macro9").name(), "Ctrl F9");
        assert_eq!(sets[0].get("Thor").name(), "F6", "unlisted actions keep defaults");
        assert_eq!(parse_keysets(&write_keysets(&sets)), sets);
        let macros = parse_macros("macro = Default\r\n1     = hello there\r\n2     = \r\n");
        assert_eq!(macros[0].lines[0], "hello there");
        assert_eq!(parse_macros(&write_macros(&macros)), macros);
    }
}
