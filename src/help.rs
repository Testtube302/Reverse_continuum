//! F1 help pages in Continuum's `Commands.hlp` format, read from the player's install.
//!
//! Each page starts with a `# Title` line. In the text, `%B` `%W` `%Y` `%G` `%R` `%p` switch the
//! colour (blue, white, yellow, green, dark red, pink); any other `%x` is printed as is (the
//! macro list uses `%tickname` and so on). The colour carries over from one line to the next.

/// Colours used by help text, matching rows of the HUD font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpColor {
    White,
    Green,
    Blue,
    DarkRed,
    Yellow,
    Pink,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HelpPage {
    pub title: String,
    /// Every line up to the next page, including the blank line usually left before it: the
    /// original sizes its box by this count.
    /// Each line as runs of same-coloured text.
    pub lines: Vec<Vec<(HelpColor, String)>>,
}

impl HelpPage {
    /// Width of the widest line (or the title) in characters.
    pub fn width_chars(&self) -> usize {
        let line = |l: &Vec<(HelpColor, String)>| l.iter().map(|(_, t)| t.chars().count()).sum::<usize>();
        self.lines.iter().map(line).chain([self.title.chars().count()]).max().unwrap_or(0)
    }
}

fn code_color(c: char) -> Option<HelpColor> {
    Some(match c {
        'W' => HelpColor::White,
        'G' => HelpColor::Green,
        'B' => HelpColor::Blue,
        'R' => HelpColor::DarkRed,
        'Y' => HelpColor::Yellow,
        'p' => HelpColor::Pink,
        _ => return None,
    })
}

/// Splits a help file into pages. Text before the first `#` line is ignored.
pub fn parse(text: &str) -> Vec<HelpPage> {
    let mut pages: Vec<HelpPage> = Vec::new();
    let mut color = HelpColor::White;
    for raw in text.lines() {
        let raw = raw.trim_end_matches('\r');
        if let Some(title) = raw.strip_prefix('#') {
            pages.push(HelpPage { title: title.trim().to_string(), lines: Vec::new() });
            color = HelpColor::White;
            continue;
        }
        let Some(page) = pages.last_mut() else { continue };
        let mut runs: Vec<(HelpColor, String)> = Vec::new();
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                if let Some(next) = chars.peek().copied().and_then(code_color) {
                    chars.next();
                    color = next;
                    continue;
                }
            }
            match runs.last_mut() {
                Some((col, s)) if *col == color => s.push(c),
                _ => runs.push((color, c.to_string())),
            }
        }
        page.lines.push(runs);
    }
    pages
}

/// Our own pages, used when the install has no Commands.hlp.
pub const FALLBACK: &str = "# Help: Controls (F1 for more)
%YFlying
Up/Down     %WThrust forward / reverse
%YLeft/Right  %WTurn
%YShift       %WAfterburner

%YWeapons
Ctrl        %WFire guns
%YTab         %WFire a bomb
%YShift+Tab   %WLay a mine
%YDelete      %WToggle multifire (once you have it)

%YShips and screens
1-8         %WChoose a ship
%YS           %WSpectate
%YEsc         %WMenu (Q quits)
%YF1          %WNext help page
%YF2          %WPlayer list
# Help: About
%YAIContinuum
%WA clean-room SubSpace client written in Rust. It reads
the game's art, sounds and help from your own Continuum
folder.
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_carry_over_and_unknown_codes_stay() {
        let pages = parse("junk\r\n# Help: Macros\r\n%pMacros\r\n%Y%tickname%W  Name\r\nnext\r\n\r\n# Two\r\nx\r\n");
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].title, "Help: Macros");
        assert_eq!(pages[0].lines.len(), 4, "the blank line before the next page counts");
        assert_eq!(pages[0].lines[0], vec![(HelpColor::Pink, "Macros".to_string())]);
        assert_eq!(
            pages[0].lines[1],
            vec![(HelpColor::Yellow, "%tickname".to_string()), (HelpColor::White, "  Name".to_string())]
        );
        assert_eq!(pages[0].lines[2], vec![(HelpColor::White, "next".to_string())]);
        assert_eq!(pages[0].width_chars(), "%tickname  Name".len());
        assert_eq!(pages[1].lines[0][0].0, HelpColor::White, "colour resets per page");
    }

    #[test]
    fn fallback_parses() {
        let pages = parse(FALLBACK);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].lines[1][0], (HelpColor::Yellow, "Up/Down     ".to_string()));
    }
}
