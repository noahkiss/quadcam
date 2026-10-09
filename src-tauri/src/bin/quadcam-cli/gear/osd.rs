//! `quadcam-cli gear osd`: an FC's OSD layout per OSD profile, drawn and checked.

use anyhow::Result;
use clap::Args;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::Core;
use quadcam_lib::gear::osd;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct OsdArgs {
    /// Betaflight `dump all`, `diff all` or CLI-line files, read in order (a later file's
    /// lines win), or one saved device id.
    #[arg(required = true)]
    pub target: Vec<String>,
    /// The grid: NTSC, PAL, HD or WxH (default: the files' vcd_video_system).
    #[arg(long)]
    pub grid: Option<String>,
    /// Print the drawn profiles as text instead of the structured view.
    #[arg(long)]
    pub text: bool,
    /// With a device id: show the layout after its staged OSD edits.
    #[arg(long)]
    pub staged: bool,
}

#[derive(Args)]
pub struct OsdEditArgs {
    /// A saved FC's device id.
    pub device: String,
    /// Move an element: `vbat=12,3` (x,y). Repeat for more.
    #[arg(long = "move", value_name = "ELEMENT=X,Y")]
    pub moves: Vec<String>,
    /// Pick the OSD profiles that show an element: `vbat=1,3`, or `vbat=none` to turn it off.
    #[arg(long = "profiles", value_name = "ELEMENT=LIST")]
    pub profiles: Vec<String>,
    /// Make profile TO show what profile FROM shows: `1:2`.
    #[arg(long, value_name = "FROM:TO")]
    pub copy: Option<String>,
}

fn pair<'a>(s: &'a str, what: &str) -> Result<(&'a str, &'a str)> {
    s.split_once('=')
        .ok_or_else(|| anyhow::anyhow!("{s:?} is not {what}"))
}

fn num(s: &str) -> Result<u8> {
    s.trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("{s:?} is not a number"))
}

pub fn edit(core: &Core, a: OsdEditArgs) -> Result<Value> {
    // One move per element: --move sets x and y, --profiles sets the profiles.
    let mut moves: Vec<osd::OsdMove> = Vec::new();
    let mut entry = |name: &str| -> usize {
        moves
            .iter()
            .position(|m| m.element == name)
            .unwrap_or_else(|| {
                moves.push(osd::OsdMove {
                    element: name.to_string(),
                    ..Default::default()
                });
                moves.len() - 1
            })
    };
    let mut picks = Vec::new();
    for m in &a.moves {
        let (el, xy) = pair(m, "ELEMENT=X,Y")?;
        let (x, y) = xy
            .split_once(',')
            .ok_or_else(|| anyhow::anyhow!("{m:?} is not ELEMENT=X,Y"))?;
        picks.push((entry(el), Some(num(x)?), Some(num(y)?), None));
    }
    for p in &a.profiles {
        let (el, list) = pair(p, "ELEMENT=LIST")?;
        let list: Vec<u8> = if list.trim() == "none" {
            Vec::new()
        } else {
            list.split(',').map(num).collect::<Result<_>>()?
        };
        picks.push((entry(el), None, None, Some(list)));
    }
    for (i, x, y, profiles) in picks {
        moves[i].x = x.or(moves[i].x);
        moves[i].y = y.or(moves[i].y);
        moves[i].profiles = profiles.or(moves[i].profiles.take());
    }
    let copy = a
        .copy
        .as_deref()
        .map(|c| -> Result<osd::OsdCopy> {
            let (f, t) = c
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("{c:?} is not FROM:TO"))?;
            Ok(osd::OsdCopy {
                from: num(f)?,
                to: num(t)?,
            })
        })
        .transpose()?;
    let change = call::gear_osd_edit(
        core,
        api::OsdEditParams {
            device: a.device,
            moves,
            copy,
            editor: None,
        },
    )?;
    Ok(serde_json::to_value(change)?)
}

pub fn run(core: &Core, a: OsdArgs) -> Result<Value> {
    // One argument that is not a file is a device id.
    let params = match a.target.as_slice() {
        [one] if !std::path::Path::new(one).exists() => api::OsdParams {
            paths: Vec::new(),
            device: Some(one.clone()),
            grid: a.grid,
            staged: a.staged,
        },
        _ => api::OsdParams {
            paths: a.target.iter().map(PathBuf::from).collect(),
            device: None,
            grid: a.grid,
            staged: false,
        },
    };
    let view = call::gear_osd(core, params)?;
    Ok(if a.text {
        Value::String(osd::render_text(&view))
    } else {
        serde_json::to_value(view)?
    })
}
