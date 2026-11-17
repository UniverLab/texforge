//! Startup banner with a terminal-width font choice.
//!
//! The large font is used when it fits the terminal; otherwise a compact
//! four-line font from the same half-block family is used. Nothing is
//! printed when stdout is not a TTY.

pub const LARGE: &str = r#"
 ███████████          █████ █████ ███████████                                     
░█░░░███░░░█         ░░███ ░░███ ░░███░░░░░░█                                     
░   ░███  ░   ██████  ░░███ ███   ░███   █ ░   ██████  ████████   ███████  ██████ 
    ░███     ███░░███  ░░█████    ░███████    ███░░███░░███░░███ ███░░███ ███░░███
    ░███    ░███████    ███░███   ░███░░░█   ░███ ░███ ░███ ░░░ ░███ ░███░███████ 
    ░███    ░███░░░    ███ ░░███  ░███  ░    ░███ ░███ ░███     ░███ ░███░███░░░  
    █████   ░░██████  █████ █████ █████      ░░██████  █████    ░░███████░░██████ 
   ░░░░░     ░░░░░░  ░░░░░ ░░░░░ ░░░░░        ░░░░░░  ░░░░░      ░░░░░███ ░░░░░░  
                                                                 ███ ░███         
                                                                ░░██████          
                                                                 ░░░░░░           
"#;

pub const COMPACT: &str = r#"
▀▀█▀▀       █   █ █▀▀▀▀
  █   ▄▀▀▀▄ ▀▄ ▄▀ █▀▀▀  ▄▀▀▀▄ █▀▀▀  ▄▀▀▀▄ ▄▀▀▀▄
  █   █▀▀▀▀ ▄▀ ▀▄ █     █   █ █▄▄   █   █ █▀▀▀▀
  ▀    ▀▀▀  █   █ ▀      ▀▀▀  ▀     ▀▀▀▀█  ▀▀▀
"#;

fn widest(s: &str) -> usize {
    s.lines().map(|l| l.chars().count()).max().unwrap_or(0)
}

/// Pick the banner font for `width` terminal columns: the large font when
/// its widest line fits, the compact one otherwise.
pub fn text(width: usize) -> &'static str {
    if width >= widest(LARGE) {
        LARGE
    } else {
        COMPACT
    }
}

/// Rows `println!("{banner}")` advances the cursor by: every newline in the
/// banner plus the one `println!` appends.
fn printed_rows(banner: &str) -> usize {
    banner.matches('\n').count() + 1
}

/// Print the banner for the current terminal width, or nothing when stdout
/// is not a TTY. Returns the rows it advanced the cursor by, so callers that
/// draw under the banner do not assume the large font's height.
pub fn print() -> usize {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
        return 0;
    }
    let width = terminal_size::terminal_size().map_or(0, |(w, _)| usize::from(w.0));
    let banner = text(width);
    println!("{banner}");
    printed_rows(banner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_threshold_is_its_widest_line() {
        assert_eq!(widest(LARGE), 82);
    }

    #[test]
    fn text_picks_large_exactly_at_threshold() {
        assert_eq!(text(widest(LARGE)), LARGE);
    }

    #[test]
    fn text_picks_compact_one_column_below_threshold() {
        assert_eq!(text(widest(LARGE) - 1), COMPACT);
    }

    #[test]
    fn text_picks_compact_on_narrow_widths() {
        assert_eq!(text(0), COMPACT);
        assert_eq!(text(40), COMPACT);
    }

    #[test]
    fn text_picks_large_on_wide_terminals() {
        assert_eq!(text(300), LARGE);
    }

    #[test]
    fn compact_fits_narrow_terminals() {
        assert!(widest(COMPACT) <= 64);
    }

    #[test]
    fn threshold_uses_large_widest_line() {
        assert_eq!(widest(LARGE), 82);
        assert_eq!(text(82), LARGE);
        assert_eq!(text(81), COMPACT);
    }

    #[test]
    fn printed_rows_matches_each_font_height() {
        assert_eq!(printed_rows(LARGE), 13);
        assert_eq!(printed_rows(COMPACT), 6);
    }
}
