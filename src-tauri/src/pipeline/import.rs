//! Running an import: name the unnamed clips, then convert, verify and cut every clip the
//! session plans, keeping the results of clips that verified before.

use crate::media::Tools;
use crate::metadata as md;
use crate::naming::NamePlanner;
use crate::pipeline::{self, Clip, ClipJob, ClipResult, DateSource, ImportSettings, Outcome};
use crate::session::Session;
use anyhow::Result;
use chrono::NaiveDate;

/// Gives each clip without a name one that is unique within its day: the aircraft and the
/// time when a radio log dated the clip, else `<default name>-1`, `-2`, and so on. The count
/// skips names that a file in the day folder or another clip in this run already has.
fn name_unnamed(jobs: &mut [ClipJob], settings: &ImportSettings, verified: &dyn Fn(usize) -> bool) {
    let base = match settings.default_name.trim() {
        "" => crate::naming::DEFAULT_NAME,
        n => n,
    };
    let mut used: std::collections::HashSet<(String, String)> = jobs
        .iter()
        .filter(|j| !j.name.trim().is_empty())
        .map(|j| (j.date.clone(), crate::naming::slug(&j.name, base)))
        .collect();
    for j in jobs.iter_mut() {
        if j.skip || verified(j.id) || !j.name.trim().is_empty() {
            continue;
        }
        let Ok(date) = NaiveDate::parse_from_str(&j.date, "%Y-%m-%d") else {
            continue;
        };
        let place = settings
            .place_folders
            .then(|| j.meta.location.as_ref().and_then(|l| l.name.as_deref()))
            .flatten();
        let dir = crate::library::day_dir(&settings.output_dir, settings.layout, date, place);
        let ext = settings.format.ext();
        let file_date = settings.name_date_format.format(date);
        let free = |name: &str, used: &std::collections::HashSet<(String, String)>| {
            !used.contains(&(j.date.clone(), crate::naming::slug(name, base)))
                && !dir
                    .join(format!(
                        "{}.{ext}",
                        crate::naming::stem(&file_date, None, name, base)
                    ))
                    .exists()
        };
        let label = [
            j.meta.profile.as_deref().unwrap_or(""),
            j.meta.aircraft.as_str(),
        ]
        .into_iter()
        .map(str::trim)
        .find(|l| !l.is_empty());
        let by_log = match (j.source, label, j.time.as_deref().and_then(|t| t.get(..5))) {
            (DateSource::Log, Some(l), Some(t)) => {
                Some(format!("{l} {t}")).filter(|n| free(n, &used))
            }
            _ => None,
        };
        let name = by_log.unwrap_or_else(|| {
            (1..)
                .map(|n| format!("{base}-{n}"))
                .find(|n| free(n, &used))
                .unwrap_or_default()
        });
        used.insert((j.date.clone(), crate::naming::slug(&name, base)));
        j.name = name;
    }
}

/// Converts and verifies every non-skipped clip. A clip that already
/// verified earlier keeps that result, so a re-run never writes a second copy.
pub fn run_import(
    tools: &Tools,
    session: &Session,
    settings: &ImportSettings,
    on_progress: &mut dyn FnMut(usize, f64, f64),
    on_result: &mut dyn FnMut(&ClipResult),
) -> Result<Vec<ClipResult>> {
    let mut jobs = session.jobs();
    for job in &mut jobs {
        let (Some(p), Some(c)) = (
            session.plans.iter().find(|p| p.id == job.id),
            session.clips.iter().find(|c| c.id == job.id),
        ) else {
            continue;
        };
        job.meta = md::resolve(
            &p.meta,
            p.log_model.as_deref(),
            c.kind,
            &settings.profiles,
            settings.default_profile.as_deref(),
            &settings.places,
            &p.moments,
            c.duration,
            p.flight.as_ref(),
        );
        job.extra = crate::library::import_items(
            &c.key,
            &c.name,
            &settings.import_id,
            job.meta.location.as_ref().and_then(|l| l.name.as_deref()),
            job.meta.profile.as_deref(),
            &p.moments
                .iter()
                .filter(|m| m.end > 0.0 && m.start < c.duration)
                .cloned()
                .collect::<Vec<_>>(),
            c.signal.as_ref().map(|s| &s.keep[..]).unwrap_or(&[]),
            p.flight.as_ref(),
        );
        // Where the time of day came from; without it the clip shows no time (noon).
        if job.time.is_some() {
            job.extra.push((
                crate::library::KEY_TIME.to_string(),
                match job.source {
                    DateSource::Log => "log",
                    DateSource::Clip => "clip",
                    _ => "manual",
                }
                .to_string(),
            ));
        }
    }
    let done = |id: usize| {
        session
            .results
            .iter()
            .rev()
            .find(|r| r.id == id && r.outcome == Outcome::Verified)
            .cloned()
    };
    name_unnamed(&mut jobs, settings, &|id| done(id).is_some());
    let todo: Vec<&Clip> = jobs
        .iter()
        .filter(|j| !j.skip && done(j.id).is_none())
        .filter_map(|j| session.clips.iter().find(|c| c.id == j.id))
        .collect();
    pipeline::preflight(settings, &todo)?;
    let mut planner = NamePlanner::new();
    let mut out = Vec::new();
    for job in &jobs {
        let Some(clip) = session.clips.iter().find(|c| c.id == job.id) else {
            continue;
        };
        let r = if job.skip {
            ClipResult {
                id: clip.id,
                outcome: Outcome::Skipped,
                output: None,
                original: None,
                size: 0,
                error: None,
                encoder: None,
                meta: None,
                cuts: Vec::new(),
                qt: Vec::new(),
            }
        } else if let Some(prev) = done(clip.id) {
            prev
        } else {
            let dur = clip.duration;
            pipeline::import_clip(tools, clip, job, settings, &mut planner, &mut |secs| {
                on_progress(clip.id, secs, dur)
            })
        };
        let mut r = r;
        if r.outcome == Outcome::Verified {
            let cuts = session
                .plans
                .iter()
                .find(|p| p.id == clip.id)
                .map(|p| p.cuts.clone())
                .unwrap_or_default();
            r.cuts = pipeline::export_cuts(tools, clip, &r, &cuts, settings, &mut planner);
        }
        on_result(&r);
        out.push(r);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{Encoder, Format};

    #[test]
    fn unnamed_clips_get_distinct_names_per_day() {
        let out = tempfile::tempdir().unwrap();
        let day = out.path().join("2026-09-30");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("2026-09-30_flight-1.mp4"), b"").unwrap();
        let settings = ImportSettings {
            output_dir: out.path().to_path_buf(),
            format: Format::Mp4,
            encoder: Encoder::Videotoolbox,
            keep_originals: false,
            add_time: false,
            default_name: "flight".into(),
            places: Vec::new(),
            profiles: Vec::new(),
            default_profile: None,
            layout: crate::library::Layout::Day,
            place_folders: false,
            import_id: String::new(),
            name_date_format: Default::default(),
        };
        let job = |id: usize, date: &str, name: &str| ClipJob {
            id,
            skip: false,
            date: date.into(),
            time: None,
            source: DateSource::Import,
            name: name.into(),
            note: String::new(),
            meta: md::Resolved::default(),
            extra: Vec::new(),
        };
        let mut logged = job(4, "2026-09-30", "");
        logged.source = DateSource::Log;
        logged.time = Some("14:03:20".into());
        logged.meta.profile = Some("Whoop".into());
        let mut jobs = vec![
            job(0, "2026-09-30", ""),
            job(1, "2026-09-30", "flight-3"),
            job(2, "2026-09-30", ""),
            job(3, "2026-10-01", ""),
            logged,
            job(5, "2026-09-30", ""),
        ];
        name_unnamed(&mut jobs, &settings, &|id| id == 5);
        let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
        // flight-1 is on disk and flight-3 is taken in this run; a verified clip keeps its name.
        assert_eq!(
            names,
            [
                "flight-2",
                "flight-3",
                "flight-4",
                "flight-1",
                "Whoop 14:03",
                ""
            ]
        );
    }
}
