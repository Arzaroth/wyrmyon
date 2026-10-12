use qrcode::{Color, QrCode};

const QUIET: usize = 1;
const INK: &str = "\x1b[38;5;255m\x1b[48;5;232m";
const RESET: &str = "\x1b[0m";

pub fn uri(code: &str) -> String {
    format!("wormhole-transfer:{code}")
}

pub fn render(code: &str, ascii: bool, coloured: bool) -> String {
    let qr = QrCode::new(uri(code)).expect("a code fits in a QR code");
    let width = qr.width();
    let colors = qr.to_colors();
    let side = width + 2 * QUIET;
    let light = |x: usize, y: usize| {
        let inside = |v: usize| (QUIET..width + QUIET).contains(&v);
        if !inside(x) || !inside(y) {
            return y < side;
        }
        colors[(y - QUIET) * width + (x - QUIET)] == Color::Light
    };
    let mut out = String::new();
    let step = if ascii { 1 } else { 2 };
    for y in (0..side).step_by(step) {
        if coloured {
            out.push_str(INK);
        }
        for x in 0..side {
            out.push_str(match (ascii, light(x, y), light(x, y + 1)) {
                (true, true, _) => "##",
                (true, false, _) => "  ",
                (false, true, true) => "█",
                (false, true, false) => "▀",
                (false, false, true) => "▄",
                (false, false, false) => " ",
            });
        }
        if coloured {
            out.push_str(RESET);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_becomes_a_square_drawn_in_light_modules() {
        let ascii = render("7-guitarist-revenge", true, false);
        let lines: Vec<&str> = ascii.lines().collect();
        let side = lines.len();
        assert!(lines.iter().all(|l| l.len() == 2 * side));
        assert_eq!(lines[0], "##".repeat(side));
        assert_eq!(lines[side - 1], "##".repeat(side));
        assert!(lines[1].starts_with("##              ##"));

        let unicode = render("7-guitarist-revenge", false, false);
        let rows: Vec<&str> = unicode.lines().collect();
        assert_eq!(rows.len(), side.div_ceil(2));
        assert!(rows[0].starts_with("█▀▀▀▀▀▀▀█"));
        assert!(rows.iter().all(|r| r.chars().count() == side));
        assert!(rows.concat().chars().all(|c| " █▀▄".contains(c)));
        assert_ne!(
            render("7-guitarist-revenge", true, false),
            render("8-other", true, false)
        );
    }
}
