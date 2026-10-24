//! The controls a setup may assign on a board: the switches, custom switches, pots and
//! sliders of the board table (`edgetx::board`), each with the kind a job checks and the type
//! `radio.yml` gives it, and the eight trims. A setup's fitted parts (`FittedSlot`) replace
//! the factory parts: a swapped switch module, or a blanking plug in a pot slot.

use super::{Control, FittedSlot, RadioControlKind};
use crate::gear::edgetx::board::{self, BoardControl, BoardControlKind};
use crate::gear::edgetx::yaml::{unquote, Doc};
use crate::gear::model::{Refusal, RefusalCode};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;

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

/// The board's controls with their factory parts; None for a board the table does not list.
pub fn controls(board: &str) -> Option<Vec<Control>> {
    fitted_controls(board, &[]).ok().flatten()
}

/// The board's controls with the fitted parts in place: a slot fitted with another switch
/// takes that switch's kind, and a blanked slot (`NONE`, `none`) is left out. None for a
/// board the table does not list; a refusal for a slot the board lacks or a type it cannot
/// take.
pub fn fitted_controls(
    board: &str,
    fitted: &[FittedSlot],
) -> Result<Option<Vec<Control>>, Refusal> {
    let Some(spec) = board::spec(Some(board)) else {
        return Ok(None);
    };
    let slots = spec.controls();
    let mut types: BTreeMap<&str, String> = BTreeMap::new();
    for f in fitted {
        let slot = slots.iter().find(|c| c.name == f.control).ok_or_else(|| {
            Refusal::new(
                RefusalCode::BadSetting,
                format!("{} is not a switch or pot slot of the {board}.", f.control),
            )
        })?;
        types.insert(f.control.as_str(), slot_type(slot.kind, &f.radio_type)?);
    }
    let mut out = Vec::new();
    for c in slots {
        let t = match types.get(c.name.as_str()) {
            Some(t) => t.clone(),
            None => factory_type(&c),
        };
        if t.eq_ignore_ascii_case("none") {
            continue;
        }
        let kind = match c.kind {
            BoardControlKind::Switch => match t.as_str() {
                "3POS" => RadioControlKind::ThreePos,
                "TOGGLE" => RadioControlKind::Momentary,
                _ => RadioControlKind::TwoPos,
            },
            BoardControlKind::Custom => RadioControlKind::Custom,
            BoardControlKind::Pot => RadioControlKind::Pot,
            BoardControlKind::Slider => RadioControlKind::Slider,
        };
        out.push(Control {
            name: c.name,
            kind,
            radio_type: (c.kind != BoardControlKind::Custom).then_some(t),
        });
    }
    out.extend(TRIMS.iter().map(|t| Control {
        name: t.to_string(),
        kind: RadioControlKind::Trim,
        radio_type: None,
    }));
    Ok(Some(out))
}

/// The type the board's factory part gives a slot in `radio.yml`.
pub fn factory_type(c: &BoardControl) -> String {
    match c.kind {
        BoardControlKind::Switch | BoardControlKind::Custom => c
            .switch_type
            .as_deref()
            .map(board::switch_type)
            .unwrap_or_else(|| "2POS".into()),
        BoardControlKind::Pot => if c.detent {
            "with_detent"
        } else {
            "without_detent"
        }
        .into(),
        BoardControlKind::Slider => "slider".into(),
    }
}

/// The types a slot may take, as `radio.yml` writes them; empty for a custom switch (each
/// model sets those).
pub fn slot_types(kind: BoardControlKind) -> &'static [&'static str] {
    match kind {
        BoardControlKind::Switch => &["2POS", "3POS", "TOGGLE", "NONE"],
        BoardControlKind::Pot => &["with_detent", "without_detent", "none"],
        BoardControlKind::Slider => &["slider", "none"],
        BoardControlKind::Custom => &[],
    }
}

/// A slot's type in the form `radio.yml` writes it, or a refusal.
pub fn slot_type(kind: BoardControlKind, raw: &str) -> Result<String, Refusal> {
    let list = slot_types(kind);
    list.iter()
        .find(|t| t.eq_ignore_ascii_case(raw.trim()))
        .map(|t| t.to_string())
        .ok_or_else(|| {
            Refusal::new(
                RefusalCode::BadSetting,
                if list.is_empty() {
                    "A customizable switch is set per model, not fitted.".to_string()
                } else {
                    format!("A slot of this kind is {}, not {raw:?}.", list.join(", "))
                },
            )
        })
}

/// One switch or pot slot as the editor shows it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
pub struct SlotRow {
    /// `SA`, `P1`.
    pub control: String,
    pub kind: BoardControlKind,
    /// The board's factory part.
    pub factory: String,
    /// What the setup says is fitted.
    pub fitted: String,
    /// What the card's `radio.yml` says now; None when it does not list the slot.
    #[serde(default)]
    pub on_radio: Option<String>,
    /// The types the slot may take.
    pub choices: Vec<String>,
}

/// The board's switch and pot slots: factory part, fitted part and what `radio.yml` says.
pub fn slot_rows(board: &str, fitted: &[FittedSlot], radio: Option<&Doc>) -> Vec<SlotRow> {
    let Some(spec) = board::spec(Some(board)) else {
        return vec![];
    };
    spec.controls()
        .into_iter()
        .filter(|c| c.kind != BoardControlKind::Custom)
        .map(|c| {
            let factory = factory_type(&c);
            let fit = fitted
                .iter()
                .find(|f| f.control == c.name)
                .and_then(|f| slot_type(c.kind, &f.radio_type).ok())
                .unwrap_or_else(|| factory.clone());
            let block = if c.kind == BoardControlKind::Switch {
                "switchConfig"
            } else {
                "potsConfig"
            };
            let on_radio = radio
                .and_then(|d| d.find(&[block, &c.name]).ok().flatten())
                .and_then(|n| n.get("type").map(|t| unquote(t.trim()).to_string()))
                .map(|t| slot_type(c.kind, &t).unwrap_or(t));
            SlotRow {
                choices: slot_types(c.kind).iter().map(|s| s.to_string()).collect(),
                control: c.name,
                kind: c.kind,
                factory,
                fitted: fit,
                on_radio,
            }
        })
        .collect()
}

/// The fitted list with one slot set: a slot set back to its factory part leaves the list.
pub fn set_fitted(
    board: &str,
    fitted: &mut Vec<FittedSlot>,
    control: &str,
    radio_type: &str,
) -> Result<(), Refusal> {
    let spec = board::spec(Some(board)).ok_or_else(|| {
        Refusal::new(
            RefusalCode::UnknownBoard,
            format!("QuadCam has no control list for the board {board:?}."),
        )
    })?;
    let slot = spec
        .controls()
        .into_iter()
        .find(|c| c.name == control && c.kind != BoardControlKind::Custom)
        .ok_or_else(|| {
            Refusal::new(
                RefusalCode::BadSetting,
                format!("{control} is not a switch or pot slot of the {board}."),
            )
        })?;
    let t = slot_type(slot.kind, radio_type)?;
    fitted.retain(|f| f.control != control);
    if t != factory_type(&slot) {
        fitted.push(FittedSlot {
            control: control.to_string(),
            radio_type: t,
        });
        let order: Vec<String> = spec.controls().into_iter().map(|c| c.name).collect();
        fitted.sort_by_key(|f| order.iter().position(|n| *n == f.control));
    }
    Ok(())
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

    #[test]
    fn fitted_parts_replace_the_factory_ones() {
        let fit = |c: &str, t: &str| FittedSlot {
            control: c.into(),
            radio_type: t.into(),
        };
        // A momentary module in SA and a blanking plug in P2.
        let g = fitted_controls("gx12", &[fit("SA", "toggle"), fit("P2", "NONE")])
            .unwrap()
            .unwrap();
        let sa = g.iter().find(|c| c.name == "SA").unwrap();
        assert_eq!(sa.kind, RadioControlKind::Momentary);
        assert_eq!(sa.radio_type.as_deref(), Some("TOGGLE"));
        assert!(!g.iter().any(|c| c.name == "P2"));
        assert!(fitted_controls("gx12", &[fit("SW1", "2POS")]).is_err());
        assert!(fitted_controls("gx12", &[fit("P1", "3POS")]).is_err());
        assert!(fitted_controls("tx16s", &[]).unwrap().is_none());

        let mut list = vec![];
        set_fitted("gx12", &mut list, "SH", "2pos").unwrap();
        set_fitted("gx12", &mut list, "SA", "TOGGLE").unwrap();
        assert_eq!(list, [fit("SA", "TOGGLE"), fit("SH", "2POS")]);
        set_fitted("gx12", &mut list, "SA", "2POS").unwrap();
        assert_eq!(list, [fit("SH", "2POS")]);

        let rows = slot_rows("gx12", &list, None);
        let sh = rows.iter().find(|r| r.control == "SH").unwrap();
        assert_eq!(
            (sh.factory.as_str(), sh.fitted.as_str()),
            ("TOGGLE", "2POS")
        );
        assert!(!rows.iter().any(|r| r.control.starts_with("SW")));
    }
}
