//! The app's native macOS menu bar. Items that act on the app send their id to the webview
//! as a `menu` event, so a menu item and its key run the same action there. The webview
//! enables, disables and checks the items to match what it shows (`menu_state`).

use std::collections::HashMap;
use std::sync::Mutex;
use tauri::menu::{
    AboutMetadata, CheckMenuItem, CheckMenuItemBuilder, IsMenuItem, Menu, MenuItem,
    MenuItemBuilder, PredefinedMenuItem, Submenu, SubmenuBuilder,
};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

const HELP_URL: &str = "https://github.com/noahkiss/quadcam#readme";

/// The menu items the webview can change.
pub struct MenuItems<R: Runtime> {
    plain: Mutex<HashMap<String, MenuItem<R>>>,
    checks: Mutex<HashMap<String, CheckMenuItem<R>>>,
    /// The Clip menu and whether its album item is in it.
    clip: Submenu<R>,
    album_shown: Mutex<bool>,
}

struct Build<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    plain: HashMap<String, MenuItem<R>>,
    checks: HashMap<String, CheckMenuItem<R>>,
}

impl<R: Runtime> Build<'_, R> {
    fn item(&mut self, id: &str, text: &str, key: Option<&str>) -> tauri::Result<MenuItem<R>> {
        let mut b = MenuItemBuilder::with_id(id, text);
        if let Some(k) = key {
            b = b.accelerator(k);
        }
        let item = b.build(self.app)?;
        self.plain.insert(id.to_string(), item.clone());
        Ok(item)
    }

    fn check(
        &mut self,
        id: &str,
        text: &str,
        key: Option<&str>,
    ) -> tauri::Result<CheckMenuItem<R>> {
        let mut b = CheckMenuItemBuilder::with_id(id, text);
        if let Some(k) = key {
            b = b.accelerator(k);
        }
        let item = b.build(self.app)?;
        self.checks.insert(id.to_string(), item.clone());
        Ok(item)
    }

    fn submenu(&self, text: &str, items: &[&dyn IsMenuItem<R>]) -> tauri::Result<Submenu<R>> {
        SubmenuBuilder::new(self.app, text).items(items).build()
    }
}

/// Builds the menu bar, sets it on the app, and routes its events to the webview.
pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let mut b = Build {
        app,
        plain: HashMap::new(),
        checks: HashMap::new(),
    };
    let sep = || PredefinedMenuItem::separator(app);
    let info = app.package_info();
    let about = AboutMetadata {
        name: Some(info.name.clone()),
        version: Some(info.version.to_string()),
        short_version: Some(crate::BUILD.to_string()),
        ..Default::default()
    };

    let settings = b.item("settings", "Settings…", Some("CmdOrCtrl+Comma"))?;
    let notices = b.item("acknowledgements", "Acknowledgements", None)?;
    let app_menu = b.submenu(
        &info.name,
        &[
            &PredefinedMenuItem::about(app, None, Some(about))?,
            &notices,
            &sep()?,
            &settings,
            &sep()?,
            &PredefinedMenuItem::services(app, None)?,
            &sep()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &sep()?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    let import = b.item("import", "Import…", Some("Shift+CmdOrCtrl+I"))?;
    let folder = b.item("open-folder", "Open Folder…", Some("CmdOrCtrl+O"))?;
    let reveal_lib = b.item(
        "reveal-library",
        "Show Library in Finder",
        Some("Alt+CmdOrCtrl+R"),
    )?;
    let file = b.submenu(
        "File",
        &[
            &import,
            &folder,
            &sep()?,
            &reveal_lib,
            &sep()?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    let undo = b.item("undo", "Undo", Some("CmdOrCtrl+Z"))?;
    let redo = b.item("redo", "Redo", Some("Shift+CmdOrCtrl+Z"))?;
    let select_all = b.item("select-all", "Select All", Some("CmdOrCtrl+A"))?;
    let edit = b.submenu(
        "Edit",
        &[
            &undo,
            &redo,
            &sep()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &select_all,
        ],
    )?;

    let stars = [
        b.item("rate-0", "No Rating", None)?,
        b.item("rate-1", "1 Star", None)?,
        b.item("rate-2", "2 Stars", None)?,
        b.item("rate-3", "3 Stars", None)?,
        b.item("rate-4", "4 Stars", None)?,
        b.item("rate-5", "5 Stars", None)?,
    ];
    let star_refs: Vec<&dyn IsMenuItem<R>> =
        stars.iter().map(|s| s as &dyn IsMenuItem<R>).collect();
    let rating = b.submenu("Rating", &star_refs)?;
    let pick = b.item("pick", "Pick", None)?;
    let reject = b.item("reject", "Reject", None)?;
    let unflag = b.item("unflag", "Unflag", None)?;
    let rename = b.item("rename", "Rename", None)?;
    let details = b.item("edit-details", "Edit Details", Some("CmdOrCtrl+I"))?;
    let reveal = b.item("reveal-clip", "Show in Finder", Some("CmdOrCtrl+R"))?;
    let share = b.item("share", "Share…", None)?;
    // Labelled with the album's name by the webview (`menu_state`); hidden without an album.
    let photos = b.item("add-photos", "Add to Album", None)?;
    let trash = b.item("trash", "Move to Trash", Some("CmdOrCtrl+Backspace"))?;
    let clip = b.submenu(
        "Clip",
        &[
            &rating,
            &pick,
            &reject,
            &unflag,
            &sep()?,
            &rename,
            &details,
            &reveal,
            &sep()?,
            &share,
            &photos,
            &sep()?,
            &trash,
        ],
    )?;

    let grid = b.check("view-grid", "as Grid", Some("CmdOrCtrl+1"))?;
    let list = b.check("view-list", "as List", Some("CmdOrCtrl+2"))?;
    let sorts = [
        b.check("sort-date", "Date", None)?,
        b.check("sort-rating", "Rating", None)?,
        b.check("sort-duration", "Duration", None)?,
        b.check("sort-name", "Name", None)?,
    ];
    let sort_refs: Vec<&dyn IsMenuItem<R>> =
        sorts.iter().map(|s| s as &dyn IsMenuItem<R>).collect();
    let sort = b.submenu("Sort By", &sort_refs)?;
    let bigger = b.item("thumb-bigger", "Larger Thumbnails", Some("CmdOrCtrl+Equal"))?;
    let smaller = b.item(
        "thumb-smaller",
        "Smaller Thumbnails",
        Some("CmdOrCtrl+Minus"),
    )?;
    let prev = b.item("prev-clip", "Previous Clip", Some("CmdOrCtrl+BracketLeft"))?;
    let next = b.item("next-clip", "Next Clip", Some("CmdOrCtrl+BracketRight"))?;
    let view = b.submenu(
        "View",
        &[
            &grid,
            &list,
            &sort,
            &sep()?,
            &bigger,
            &smaller,
            &sep()?,
            &prev,
            &next,
            &sep()?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;

    let window = b.submenu(
        "Window",
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &sep()?,
            &PredefinedMenuItem::bring_all_to_front(app, None)?,
        ],
    )?;
    let help_item = b.item("help", "QuadCam Help", None)?;
    let help = b.submenu("Help", &[&help_item])?;

    let menu = Menu::with_items(
        app,
        &[&app_menu, &file, &edit, &clip, &view, &window, &help],
    )?;
    app.set_menu(menu)?;
    #[cfg(target_os = "macos")]
    {
        window.set_as_windows_menu_for_nsapp()?;
        help.set_as_help_menu_for_nsapp()?;
    }
    app.manage(MenuItems {
        plain: Mutex::new(b.plain),
        checks: Mutex::new(b.checks),
        clip,
        album_shown: Mutex::new(true),
    });
    app.on_menu_event(|app, event| {
        let id = event.id().0.as_str();
        if id == "help" {
            use tauri_plugin_opener::OpenerExt;
            let _ = app.opener().open_url(HELP_URL, None::<&str>);
            return;
        }
        let _ = app.emit(<crate::api::Menu as tauri_specta::Event>::NAME, id);
    });
    Ok(())
}

/// The webview's view of which items apply now: `enabled` and `checked`, by item id, and
/// the album item's label (`None` hides it).
#[tauri::command]
#[specta::specta]
pub fn menu_state(
    items: State<'_, MenuItems<tauri::Wry>>,
    enabled: HashMap<String, bool>,
    checked: HashMap<String, bool>,
    album_item: Option<String>,
) {
    let plain = items.plain.lock().unwrap();
    if let (Some(photos), Some(share)) = (plain.get("add-photos"), plain.get("share")) {
        let mut shown = items.album_shown.lock().unwrap();
        match &album_item {
            Some(label) => {
                let _ = photos.set_text(label);
                if !*shown {
                    let at = items
                        .clip
                        .items()
                        .ok()
                        .and_then(|all| all.iter().position(|i| i.id() == share.id()));
                    if let Some(at) = at {
                        *shown = items.clip.insert(photos, at + 1).is_ok();
                    }
                }
            }
            None if *shown => *shown = items.clip.remove(photos).is_err(),
            None => {}
        }
    }
    let checks = items.checks.lock().unwrap();
    for (id, on) in enabled {
        if let Some(i) = plain.get(&id) {
            let _ = i.set_enabled(on);
        } else if let Some(i) = checks.get(&id) {
            let _ = i.set_enabled(on);
        }
    }
    for (id, on) in checked {
        if let Some(i) = checks.get(&id) {
            let _ = i.set_checked(on);
        }
    }
}
