//! `quadcam-cli gear model`: a radio's model for the editors, and `model-edit`, which stages
//! timers, value screens, logging, alarms, callouts and the checklist as one "Model edits"
//! change. Nothing is written to the card; the apply sheet writes it.

use anyhow::{bail, Context, Result};
use clap::Args;
use quadcam_lib::api::{self, call};
use quadcam_lib::core::{model_edit, Core};
use quadcam_lib::gear::edgetx::editors::{CalloutDef, CalloutWhen, LoggingDef, SensorLog};
use quadcam_lib::gear::edgetx::model::{Field, ModelOp};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Args)]
pub struct ModelArgs {
    /// A saved radio's device id.
    pub device: String,
    /// The model file (model01.yml); default the radio's selected model.
    #[arg(long)]
    pub model: Option<String>,
    /// Without the device's staged model edits on top.
    #[arg(long)]
    pub no_staged: bool,
    /// Print the model as text instead of the structured view.
    #[arg(long)]
    pub text: bool,
}

#[derive(Args)]
pub struct ModelEditArgs {
    /// A saved radio's device id.
    pub device: String,
    /// The model file (model01.yml).
    #[arg(long)]
    pub model: String,
    /// A timer: `N:key=value,key=value` with N 1-3 and keys such as name, mode, swtch,
    /// minuteBeep, countdownBeep, persistent, countdownStart, showElapsed, extraHaptic.
    /// Repeat for more.
    #[arg(long, value_name = "N:KEY=VALUE,...")]
    pub timer: Vec<String>,
    /// Remove timer N.
    #[arg(long, value_name = "N")]
    pub timer_off: Vec<u32>,
    /// A value screen: `N:SRC,SRC/SRC,SRC` with N 1-4; `/` starts a line; a source is
    /// `{RxBt}` (a sensor label), `Tmr1`, ...
    #[arg(long, value_name = "N:LINES")]
    pub screen: Vec<String>,
    /// A script screen: `N:NAME` (a script in SCRIPTS/TELEMETRY, 6 characters at most).
    #[arg(long, value_name = "N:NAME")]
    pub screen_script: Vec<String>,
    /// Remove telemetry screen N.
    #[arg(long, value_name = "N")]
    pub screen_off: Vec<u32>,
    /// Logging: `SWITCH:SECONDS` (`ON:1`, `SA2:0.5`), or `off` to remove it.
    #[arg(long, value_name = "SWITCH:SECONDS|off")]
    pub logging: Option<String>,
    /// Whether the log records a sensor: `RxBt=on`, `Capa=off`. Repeat for more.
    #[arg(long, value_name = "LABEL=on|off")]
    pub log_sensor: Vec<String>,
    /// RSSI alarm levels: `WARNING:CRITICAL`.
    #[arg(long, value_name = "WARNING:CRITICAL")]
    pub rf_alarm: Option<String>,
    /// A callout: `TRACK:below:SOURCE:VALUE[:DELAY_S[:REPEAT]]`, `TRACK:above:...` or
    /// `TRACK:switch:SWITCH[:REPEAT]`. SOURCE is `{RxBt}` for a sensor. REPEAT is `1x`,
    /// `!1x` or seconds. The track is a sound in SOUNDS/<lang>/. Repeat for more.
    #[arg(long, value_name = "TRACK:WHEN:...")]
    pub callout: Vec<String>,
    /// Remove the callout with this track.
    #[arg(long, value_name = "TRACK")]
    pub callout_off: Vec<String>,
    /// The power-on checklist from a text file (`-` for stdin); `=` starts a tick box.
    #[arg(long, value_name = "FILE")]
    pub checklist_file: Option<PathBuf>,
    /// Turn the checklist off or on.
    #[arg(long, value_name = "on|off")]
    pub checklist: Option<String>,
    /// Model ops as JSON: a file holding a list of ops.
    #[arg(long, value_name = "FILE")]
    pub ops: Option<PathBuf>,
}

fn num<T: std::str::FromStr>(s: &str, what: &str) -> Result<T> {
    s.trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("{s:?} is not {what}"))
}

fn split_n<'a>(s: &'a str, what: &str) -> Result<(u32, &'a str)> {
    let (n, rest) = s
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("{s:?} is not N:{what}"))?;
    Ok((num(n, "a number")?, rest))
}

fn index(n: u32, max: u32, what: &str) -> Result<u32> {
    if n == 0 || n > max {
        bail!("{what} {n}: they run 1-{max}.");
    }
    Ok(n - 1)
}

fn seconds_to_ds(s: &str) -> Result<u32> {
    let v: f64 = num(s, "a number of seconds")?;
    if v < 0.0 {
        bail!("{s:?} is negative.");
    }
    Ok((v * 10.0).round() as u32)
}

fn onoff(s: &str) -> Result<bool> {
    match s.trim() {
        "on" | "1" | "true" => Ok(true),
        "off" | "0" | "false" => Ok(false),
        other => bail!("{other:?} is not on or off."),
    }
}

/// Turns the flags into model ops.
pub fn ops_of(a: &ModelEditArgs) -> Result<Vec<ModelOp>> {
    let mut ops = Vec::new();
    for t in &a.timer {
        let (n, rest) = split_n(t, "KEY=VALUE,...")?;
        let mut fields = Vec::new();
        for kv in rest.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("{kv:?} is not KEY=VALUE"))?;
            fields.push(Field::new(k.trim(), v.trim()));
        }
        ops.push(ModelOp::SetTimer {
            index: index(n, 3, "Timer")?,
            fields,
        });
    }
    for n in &a.timer_off {
        ops.push(ModelOp::RemoveTimer {
            index: index(*n, 3, "Timer")?,
        });
    }
    for s in &a.screen {
        let (n, rest) = split_n(s, "LINES")?;
        let lines = rest
            .split('/')
            .map(|l| {
                l.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .collect();
        ops.push(ModelOp::SetScreenValues {
            index: index(n, 4, "Screen")?,
            lines,
        });
    }
    for s in &a.screen_script {
        let (n, name) = split_n(s, "NAME")?;
        ops.push(ModelOp::SetScreen {
            index: index(n, 4, "Screen")?,
            script: Some(name.trim().to_string()),
        });
    }
    for n in &a.screen_off {
        ops.push(ModelOp::SetScreen {
            index: index(*n, 4, "Screen")?,
            script: None,
        });
    }
    if let Some(l) = &a.logging {
        ops.push(ModelOp::SetLogging {
            logging: if l.trim() == "off" {
                None
            } else {
                let (sw, secs) = l
                    .rsplit_once(':')
                    .ok_or_else(|| anyhow::anyhow!("{l:?} is not SWITCH:SECONDS"))?;
                Some(LoggingDef {
                    swtch: sw.trim().to_string(),
                    period_ds: seconds_to_ds(secs)?,
                })
            },
        });
    }
    if !a.log_sensor.is_empty() {
        let mut sensors = Vec::new();
        for s in &a.log_sensor {
            let (label, v) = s
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("{s:?} is not LABEL=on|off"))?;
            sensors.push(SensorLog {
                label: label.trim().to_string(),
                logs: onoff(v)?,
            });
        }
        ops.push(ModelOp::SetSensorLogs { sensors });
    }
    if let Some(r) = &a.rf_alarm {
        let (w, c) = r
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("{r:?} is not WARNING:CRITICAL"))?;
        ops.push(ModelOp::SetRfAlarms {
            warning: num(w, "a number")?,
            critical: num(c, "a number")?,
        });
    }
    for c in &a.callout {
        ops.push(callout_op(c)?);
    }
    for t in &a.callout_off {
        ops.push(ModelOp::RemoveCallout { track: t.clone() });
    }
    if let Some(c) = &a.checklist {
        ops.push(ModelOp::SetChecklist { enabled: onoff(c)? });
    }
    if let Some(f) = &a.ops {
        let text =
            std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
        let more: Vec<ModelOp> =
            serde_json::from_str(&text).context("the ops file is not a list of model ops")?;
        ops.extend(more);
    }
    Ok(ops)
}

fn callout_op(c: &str) -> Result<ModelOp> {
    let parts: Vec<&str> = c.split(':').collect();
    let bad = || {
        anyhow::anyhow!("{c:?} is not TRACK:below|above:SOURCE:VALUE[:DELAY_S[:REPEAT]] or TRACK:switch:SWITCH[:REPEAT]")
    };
    let (track, kind) = (
        parts.first().ok_or_else(bad)?,
        parts.get(1).ok_or_else(bad)?,
    );
    let (when, repeat) = match *kind {
        "switch" => (
            CalloutWhen::Switch {
                swtch: parts.get(2).ok_or_else(bad)?.to_string(),
            },
            parts.get(3).map(|s| s.to_string()),
        ),
        "below" | "above" => {
            let source = parts.get(2).ok_or_else(bad)?.to_string();
            let value = parts.get(3).ok_or_else(bad)?.to_string();
            let delay_ds = parts
                .get(4)
                .map(|d| seconds_to_ds(d))
                .transpose()?
                .unwrap_or(0);
            let repeat = parts.get(5).map(|s| s.to_string());
            (
                if *kind == "below" {
                    CalloutWhen::Below {
                        source,
                        value,
                        delay_ds,
                    }
                } else {
                    CalloutWhen::Above {
                        source,
                        value,
                        delay_ds,
                    }
                },
                repeat,
            )
        }
        _ => return Err(bad()),
    };
    Ok(ModelOp::SetCallout {
        callout: CalloutDef {
            track: track.to_string(),
            when,
            repeat,
        },
    })
}

pub fn run(core: &Core, a: ModelArgs) -> Result<Value> {
    let d = call::gear_model(
        core,
        api::ModelParams {
            device: a.device,
            model: a.model,
            staged: !a.no_staged,
        },
    )?;
    Ok(if a.text {
        Value::String(model_edit::render_text(&d))
    } else {
        serde_json::to_value(d)?
    })
}

pub fn edit(core: &Core, a: ModelEditArgs) -> Result<Value> {
    let ops = ops_of(&a)?;
    let checklist = match &a.checklist_file {
        None => None,
        Some(p) if p.as_os_str() == "-" => {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
            Some(s)
        }
        Some(p) => {
            Some(std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?)
        }
    };
    let change = call::gear_model_edit(
        core,
        api::ModelEditParams {
            device: a.device,
            model: a.model,
            ops,
            checklist,
            editor: None,
        },
    )?;
    Ok(serde_json::to_value(change)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> ModelEditArgs {
        ModelEditArgs {
            device: "d".into(),
            model: "model01.yml".into(),
            timer: vec![],
            timer_off: vec![],
            screen: vec![],
            screen_script: vec![],
            screen_off: vec![],
            logging: None,
            log_sensor: vec![],
            rf_alarm: None,
            callout: vec![],
            callout_off: vec![],
            checklist_file: None,
            checklist: None,
            ops: None,
        }
    }

    #[test]
    fn flags_become_ops() {
        let mut a = args();
        a.timer = vec!["2:name=FLT,minuteBeep=1".into()];
        a.timer_off = vec![3];
        a.screen = vec!["2:{RxBt},Tmr1/{Capa}".into()];
        a.logging = Some("SA2:0.5".into());
        a.log_sensor = vec!["RxBt=off".into()];
        a.rf_alarm = Some("50:40".into());
        a.callout = vec![
            "lowbat:below:{RxBt}:3.5:2:5".into(),
            "armed:switch:L1:!1x".into(),
        ];
        let ops = ops_of(&a).unwrap();
        assert_eq!(ops.len(), 8);
        assert_eq!(
            ops[0],
            ModelOp::SetTimer {
                index: 1,
                fields: vec![Field::new("name", "FLT"), Field::new("minuteBeep", "1")]
            }
        );
        assert_eq!(ops[1], ModelOp::RemoveTimer { index: 2 });
        assert_eq!(
            ops[2],
            ModelOp::SetScreenValues {
                index: 1,
                lines: vec![vec!["{RxBt}".into(), "Tmr1".into()], vec!["{Capa}".into()]]
            }
        );
        assert_eq!(
            ops[3],
            ModelOp::SetLogging {
                logging: Some(LoggingDef {
                    swtch: "SA2".into(),
                    period_ds: 5
                })
            }
        );
        assert_eq!(
            ops[6],
            ModelOp::SetCallout {
                callout: CalloutDef {
                    track: "lowbat".into(),
                    when: CalloutWhen::Below {
                        source: "{RxBt}".into(),
                        value: "3.5".into(),
                        delay_ds: 20
                    },
                    repeat: Some("5".into())
                }
            }
        );
    }

    #[test]
    fn bad_flags_say_what_is_wrong() {
        let mut a = args();
        a.timer = vec!["4:name=X".into()];
        assert!(ops_of(&a).unwrap_err().to_string().contains("1-3"));
        let mut a = args();
        a.callout = vec!["lowbat:sometimes:x".into()];
        assert!(ops_of(&a).unwrap_err().to_string().contains("TRACK"));
        let mut a = args();
        a.logging = Some("SA2".into());
        assert!(ops_of(&a)
            .unwrap_err()
            .to_string()
            .contains("SWITCH:SECONDS"));
    }
}
