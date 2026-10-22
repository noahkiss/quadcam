//! QuadCam's own telemetry screens for 128x64 radios, written in Lua by QuadCam (MIT). The
//! setup renders each with its settings (the finder's control, the timer the volt screen
//! shows). Each file starts with `MARK`, so QuadCam knows its own scripts: it replaces one of
//! its own that differs, and leaves a script of the same name that someone else wrote.

use super::{LuaKind, LuaScreen};

/// The first line of every script QuadCam writes.
pub const MARK: &str = "-- QuadCam telemetry screen:";

const FINDER: &str = include_str!("finder.lua");
const VOLT: &str = include_str!("volt.lua");

/// True when these bytes are a script QuadCam wrote.
pub fn is_quadcam(bytes: &[u8]) -> bool {
    bytes.starts_with(MARK.as_bytes())
}

/// The finder control as the script reads it: the switch's name, and whether its on
/// position reads +1024 (position 2) or -1024 (position 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinderControl {
    /// `SD`.
    pub switch: String,
    /// 0 or 2.
    pub on_position: u8,
}

/// The script's text. `finder` is needed for a finder screen.
pub fn render(screen: &LuaScreen, finder: Option<&FinderControl>) -> Option<String> {
    match &screen.kind {
        LuaKind::Volt { timer } => Some(
            VOLT.replace("@timer_number@", &(timer + 1).to_string())
                .replace("@timer@", &timer.to_string()),
        ),
        LuaKind::Finder { .. } => {
            let f = finder?;
            let sign = if f.on_position == 0 { "-1" } else { "1" };
            Some(
                FINDER
                    .replace("@switch@", &f.switch.to_ascii_lowercase())
                    .replace("@SWITCH@", &f.switch)
                    .replace("@sign@", sign),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_render_with_their_settings_and_carry_the_mark() {
        let finder = LuaScreen {
            name: "finder".into(),
            kind: LuaKind::Finder {
                job: "finder".into(),
            },
        };
        let t = render(
            &finder,
            Some(&FinderControl {
                switch: "SD".into(),
                on_position: 2,
            }),
        )
        .unwrap();
        assert!(is_quadcam(t.as_bytes()));
        assert!(t.contains("local SWITCH = \"sd\""));
        assert!(t.contains("local ON_SIGN = 1 "));
        assert!(t.contains("\"SD on=beep\""));
        assert!(!t.contains('@'), "every token is filled");
        let t = render(
            &finder,
            Some(&FinderControl {
                switch: "SF".into(),
                on_position: 0,
            }),
        )
        .unwrap();
        assert!(t.contains("local ON_SIGN = -1 "));
        assert!(render(&finder, None).is_none());
        let volt = LuaScreen {
            name: "volt".into(),
            kind: LuaKind::Volt { timer: 1 },
        };
        let t = render(&volt, None).unwrap();
        assert!(t.contains("local TIMER = 1\n"));
        assert!(t.contains("Timer 2."));
        assert!(!t.contains('@'));
        assert!(!is_quadcam(b"-- finder.lua: someone's own script"));
    }

    #[test]
    fn scripts_use_lua_5_2_only() {
        // EdgeTX runs Lua 5.2: no integer division or bit operators.
        for s in [FINDER, VOLT] {
            for line in s.lines().filter(|l| !l.trim_start().starts_with("--")) {
                assert!(!line.contains("//"), "{line}");
                assert!(!line.contains(">>") && !line.contains("<<"), "{line}");
            }
        }
    }
}
