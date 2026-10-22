//! The controls a setup may assign on a board: the switches, custom switches, pots and
//! sliders of the board table (`edgetx::board`), each with the kind a job checks and the type
//! `radio.yml` gives it, and the eight trims.

use super::{Control, RadioControlKind};
use crate::gear::edgetx::board::{self, BoardControlKind};

/// The eight trim directions, as EdgeTX names them in a model file.
pub const TRIMS: &[&str] = &[
    "TrimRudLeft",
    "TrimRudRight",
    "TrimEleDown",
    "TrimEleUp",
    "TrimThrDown",
    "TrimThrUp",
    "TrimAilLeft",
    "TrimAilRight",
];

/// The board's controls; None for a board the table does not list.
pub fn controls(board: &str) -> Option<Vec<Control>> {
    let spec = board::spec(Some(board))?;
    let mut out: Vec<Control> = spec
        .controls()
        .into_iter()
        .map(|c| {
            let t = c.switch_type.as_deref().map(board::switch_type);
            let (kind, radio_type) = match c.kind {
                BoardControlKind::Switch => (
                    match t.as_deref() {
                        Some("3POS") => RadioControlKind::ThreePos,
                        Some("TOGGLE") => RadioControlKind::Momentary,
                        _ => RadioControlKind::TwoPos,
                    },
                    t,
                ),
                BoardControlKind::Custom => (RadioControlKind::Custom, None),
                BoardControlKind::Pot => (
                    RadioControlKind::Pot,
                    Some(
                        if c.detent {
                            "with_detent"
                        } else {
                            "without_detent"
                        }
                        .into(),
                    ),
                ),
                BoardControlKind::Slider => (RadioControlKind::Slider, Some("slider".into())),
            };
            Control {
                name: c.name,
                kind,
                radio_type,
            }
        })
        .collect();
    out.extend(TRIMS.iter().map(|t| Control {
        name: t.to_string(),
        kind: RadioControlKind::Trim,
        radio_type: None,
    }));
    Some(out)
}

/// True for a control `radio.yml` lists under `switchConfig`.
pub fn is_switch(control: &Control) -> bool {
    matches!(
        control.kind,
        RadioControlKind::TwoPos | RadioControlKind::ThreePos | RadioControlKind::Momentary
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boards_list_their_controls() {
        let p = controls("pocket").unwrap();
        assert_eq!(
            p.iter()
                .filter(|c| c.kind == RadioControlKind::Trim)
                .count(),
            8
        );
        let sa = p.iter().find(|c| c.name == "SA").unwrap();
        assert_eq!(sa.kind, RadioControlKind::TwoPos);
        let se = p.iter().find(|c| c.name == "SE").unwrap();
        assert_eq!(se.kind, RadioControlKind::Momentary);
        let p1 = p.iter().find(|c| c.name == "P1").unwrap();
        assert_eq!(p1.radio_type.as_deref(), Some("without_detent"));
        let g = controls("GX12").unwrap();
        assert_eq!(
            g.iter()
                .filter(|c| c.kind == RadioControlKind::Custom)
                .count(),
            6
        );
        assert_eq!(
            g.iter().find(|c| c.name == "SH").unwrap().kind,
            RadioControlKind::Momentary
        );
        assert_eq!(
            g.iter()
                .find(|c| c.name == "P2")
                .unwrap()
                .radio_type
                .as_deref(),
            Some("with_detent")
        );
        assert!(controls("tx16s").is_none());
    }
}
