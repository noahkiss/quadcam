use super::*;

#[test]
fn boards_are_unique_lowercase_and_found_ignoring_case() {
    for (i, p) in PROFILES.iter().enumerate() {
        assert_eq!(p.board, p.board.to_ascii_lowercase());
        assert!(PROFILES[i + 1..].iter().all(|q| q.board != p.board));
        assert_eq!(profile(&format!(" {} ", p.board.to_uppercase())), Some(p));
        if let Some(f) = &p.flash {
            assert_eq!(f.id, p.board, "the flash spec is the board's own");
            assert!(f.names.contains(&p.board));
            assert!(f.min_kb < f.max_kb && f.max_kb * 1024 <= f.flash_bytes);
        }
        assert_eq!(p.usb_product, p.usb_product.to_ascii_lowercase());
        // A board's own product finds the board, however long a look-alike's name is.
        assert_eq!(
            by_usb_product(&format!("{} Mass Storage", p.usb_product)).map(|q| q.board),
            Some(p.board),
            "{}",
            p.board
        );
        if p.dfu.start == DfuStart::Unknown {
            assert!(
                p.dfu.steps.is_empty() && p.dfu.hint.is_empty(),
                "{}",
                p.board
            );
        } else {
            assert!(
                !p.dfu.steps.is_empty() && !p.dfu.hint.is_empty(),
                "{}",
                p.board
            );
        }
        assert!(!p.sd_pack.is_empty() && !p.yaml_schema.is_empty());
        for (j, s) in p.slots.iter().enumerate() {
            assert!(
                p.slots[j + 1..].iter().all(|t| t.name != s.name),
                "{}",
                p.board
            );
        }
    }
    assert!(profile("tx16s").is_some_and(|p| !p.proven()));
    assert!(profile("nope").is_none());
}

#[test]
fn the_chip_decides_the_format_and_the_last_series() {
    for p in PROFILES {
        // Every H7 board updates by UF2; STM32F2 boards stop at EdgeTX 2.11.
        assert_eq!(
            p.firmware_format == FirmwareFormat::Uf2,
            p.mcu.starts_with("STM32H7"),
            "{}",
            p.board
        );
        let f2 = p.mcu.starts_with("STM32F2");
        assert_eq!(
            p.edgetx_series,
            if f2 { "2.11" } else { "2.12" },
            "{}",
            p.board
        );
        // A chip a card overrides has no write route until one is tested.
        assert_eq!(
            p.write_route == WriteRoute::Untested,
            p.storage == StorageKind::ChipCardOverrides
        );
    }
    for b in [
        "tx12", "tlite", "tpro", "t12", "lr3pro", "x9d", "x9dp", "xlite", "xlites",
    ] {
        assert_eq!(profile(b).unwrap().edgetx_series, "2.11", "{b}");
    }
    for b in [
        "tx15", "gx15", "tx16smk3", "t15pro", "t22", "v12", "v15", "pa01", "st16", "c14",
    ] {
        assert_eq!(
            profile(b).unwrap().firmware_format,
            FirmwareFormat::Uf2,
            "{b}"
        );
    }
}

#[test]
fn only_proven_boards_flash() {
    for p in PROFILES {
        assert_eq!(p.flash.is_some(), p.proven(), "{}", p.board);
    }
    let proven: Vec<&str> = PROFILES
        .iter()
        .filter(|p| p.proven())
        .map(|p| p.board)
        .collect();
    assert_eq!(proven, ["pocket", "gx12"]);
}

#[test]
fn the_gx12_row() {
    let g = profile("gx12").unwrap();
    assert_eq!(g.mcu, "STM32F407xG");
    assert_eq!(g.firmware_format, FirmwareFormat::Bin);
    assert_eq!(g.dfu.start, DfuStart::ButtonCombo);
    assert!(g.dfu.hint.contains("Hold SG"));
    assert_eq!(g.storage, StorageKind::Chip);
    assert_eq!(g.write_route, WriteRoute::UsbStorage);
    assert_eq!(g.yaml_schema, "gx12");
    assert_eq!(g.sd_pack, "bw128x64");
    assert_eq!(
        g.usb_speed,
        Some(UsbSpeed {
            write_kb_s: 360,
            read_kb_s: 370
        })
    );
    let e = g.elrs_target().unwrap();
    assert_eq!(e.target, "radiomaster.tx_dual.gx12");
    assert!(!e.upload_methods.contains(&"uart"));
    assert!(e.upload_methods.contains(&"etx"));
    let f = g.flash.unwrap();
    assert_eq!(f.flash_bytes, 0x10_0000);
    let p = profile("pocket").unwrap();
    assert_eq!(p.dfu.start, DfuStart::PlugIn);
    assert_eq!(p.storage, StorageKind::CardSlot);
    assert_eq!(p.yaml_schema, "128x64");
    assert_eq!(
        p.elrs_target().unwrap().target,
        "radiomaster.tx_2400.pocket"
    );
}

#[test]
fn slots_hold_what_the_hardware_takes() {
    let g = profile("gx12").unwrap();
    let names: Vec<&str> = g.slots.iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        [
            "SA", "SB", "SC", "SD", "SE", "SF", "SG", "SH", "SW1", "SW2", "SW3", "SW4", "SW5",
            "SW6", "P1", "P2", "SL1", "SL2"
        ]
    );
    let sb = g.slots.iter().find(|s| s.name == "SB").unwrap();
    assert_eq!((sb.kind, sb.hardware), (BoardControlKind::Switch, "3POS"));
    let sw1 = g.slots.iter().find(|s| s.name == "SW1").unwrap();
    assert_eq!(sw1.kind, BoardControlKind::Custom);
    let p1 = g.slots.iter().find(|s| s.name == "P1").unwrap();
    assert_eq!(
        (p1.kind, p1.default),
        (BoardControlKind::Pot, Some("POT_CENTER"))
    );
    // The Pocket has one pot slot, the wheel.
    let pocket = profile("pocket").unwrap();
    assert_eq!(
        pocket
            .slots
            .iter()
            .filter(|s| s.kind == BoardControlKind::Pot)
            .count(),
        1
    );
    assert_eq!(g.trims, 4);
}

#[test]
fn usb_products_match_ignoring_case_and_only_whole_models() {
    let board = |s: &str| by_usb_product(s).map(|p| p.board);
    assert_eq!(board("Radiomaster GX12 Mass Storage"), Some("gx12"));
    assert_eq!(board("RadioMaster GX12 Serial Port"), Some("gx12"));
    assert_eq!(board("RadioMaster Pocket Joystick"), Some("pocket"));
    assert_eq!(board("RadioMaster GX120 DFU"), None);
    assert_eq!(board("RadioMaster TX16S Mass Storage"), Some("tx16s"));
    // The longest name wins: a look-alike's product starts with the other's.
    assert_eq!(board("RadioMaster TX16S MK3 DFU"), Some("tx16smk3"));
    assert_eq!(board("Jumper T20 V2 Mass Storage"), Some("t20v2"));
    assert_eq!(board("Jumper T20 Mass Storage"), Some("t20"));
    assert_eq!(board("Jumper T-Pro V2 Joystick"), Some("tprov2"));
    assert_eq!(board("Jumper T-Pro Joystick"), Some("tpro"));
    assert_eq!(
        board("RadioMaster TX12 Mark II Serial Port"),
        Some("tx12mk2")
    );
    assert_eq!(board("HelloRadioSky V14LCD Mass Storage"), Some("v14lcd"));
    assert_eq!(board("HelloRadioSky V14 Mass Storage"), Some("v14"));
    assert_eq!(board("HelloRadioSky V15 Bootloader"), Some("v15"));
    assert_eq!(board("STM32  BOOTLOADER"), None);
}

#[test]
fn the_dfu_hint_follows_the_board() {
    assert!(dfu_hint(Some("pocket")).contains("do not hold the trim buttons"));
    assert!(!dfu_hint(Some("pocket")).contains("SG"));
    assert!(dfu_hint(Some("GX12")).contains("Hold SG"));
    assert!(dfu_hint(Some("t20v2")).contains("toothpick"));
    // An unknown board, or none, hears the proven ways in, not every row's.
    for b in [None, Some("no-such-board")] {
        let h = dfu_hint(b);
        assert!(h.contains("do not hold the trim buttons"), "{h}");
        assert!(
            h.contains("RadioMaster GX12") && h.contains("Hold SG"),
            "{h}"
        );
        assert!(!h.contains("toothpick"), "{h}");
    }
    // A board whose way in no source gives says so.
    let h = dfu_hint(Some("v15"));
    assert!(h.contains("does not know how the HelloRadioSky V15"), "{h}");
    assert!(dfu_steps(Some("v15")).is_empty());
    assert_eq!(dfu_steps(Some("gx12")).len(), 4);
    assert_eq!(dfu_steps(Some("tx16s")).len(), 3);
    assert!(dfu_steps(Some("no-such-board")).is_empty());
}

#[test]
fn the_flash_guards_refuse_a_later_series() {
    assert!(check_flash(Some("gx12"), "2.12.5").is_ok());
    assert!(check_flash(Some("gx12"), "2.11.0").is_ok());
    let e = check_flash(Some("gx12"), "2.13.0").unwrap_err();
    assert_eq!(e.code, RefusalCode::Incompatible);
    assert!(e.reason.contains("up to 2.12"), "{}", e.reason);
    assert!(
        check_flash(Some("no-such-board"), "9.9.9").is_ok(),
        "compat refuses it"
    );
    assert!(check_flash(None, "2.12.5").is_ok());
}

#[test]
fn an_f2_board_stops_at_2_11() {
    for b in ["tx12", "tlite", "t12", "tpro", "x9dp"] {
        assert!(check_flash(Some(b), "2.11.8").is_ok(), "{b}");
        let e = check_flash(Some(b), "2.12.0").unwrap_err();
        assert!(e.reason.contains("up to 2.11"), "{b}: {}", e.reason);
    }
    // Its F4 look-alike goes on.
    assert!(check_flash(Some("tx12mk2"), "2.12.5").is_ok());
    assert!(check_flash(Some("x9dp2019"), "2.12.5").is_ok());
}

#[test]
fn a_uf2_board_refuses_dfu_and_names_the_uf2_route() {
    for b in ["tx15", "gx15", "tx16smk3", "t15pro", "t22", "v15", "pa01"] {
        let e = check_flash(Some(b), "2.12.5").unwrap_err();
        assert_eq!(e.code, RefusalCode::Incompatible, "{b}");
        assert!(
            e.reason.contains("UF2 file, not over DFU"),
            "{b}: {}",
            e.reason
        );
        assert!(e.reason.contains("EDGETX_UF2"), "{b}: {}", e.reason);
        assert!(e.reason.contains(&format!("{b} .uf2")), "{b}: {}", e.reason);
    }
    let tx15 = check_flash(Some("tx15"), "2.12.5").unwrap_err();
    assert!(
        tx15.reason.contains("Hold both horizontal trims inward"),
        "{}",
        tx15.reason
    );
    // A UF2 board with no known bootloader entry still names the route.
    let c14 = check_flash(Some("c14"), "2.12.5").unwrap_err();
    assert!(
        c14.reason.contains("the maker's manual says how"),
        "{}",
        c14.reason
    );
}

#[test]
fn v12_firmware_before_2_12_3_is_another_radio() {
    for v in ["2.11.4", "2.12.0", "2.12.2", "v2.12.2"] {
        let e = check_flash(Some("v12"), v).unwrap_err();
        assert!(e.reason.contains("another radio"), "{v}: {}", e.reason);
        assert!(!e.reason.contains("UF2"), "{v}: the floor speaks first");
    }
    // From 2.12.3 the target is the HelloRadioSky V12; it still updates by UF2.
    for v in ["2.12.3", "2.12.5"] {
        let e = check_flash(Some("v12"), v).unwrap_err();
        assert!(e.reason.contains("UF2"), "{v}: {}", e.reason);
    }
    let w = profile("v12").unwrap().warnings();
    assert!(w.iter().any(|x| x.contains("older than 2.12.3")), "{w:?}");
}

#[test]
fn look_alikes_warn_and_point_at_real_boards() {
    for p in PROFILES {
        for l in p.look_alikes {
            if let Some(b) = l.board {
                let other = profile(b).unwrap_or_else(|| panic!("{} names {b}", p.board));
                assert_ne!(other.board, p.board);
                assert!(
                    other.look_alikes.iter().any(|m| m.board == Some(p.board)),
                    "{} and {b} name each other",
                    p.board
                );
            }
            assert!(p.warnings().iter().any(|w| w == l.note));
        }
    }
    let warns = |b: &str| profile(b).unwrap().warnings().join("\n");
    assert!(warns("t20").contains("T20 V2"));
    assert!(warns("t20v2").contains("not t20"));
    assert!(warns("tpro").contains("mirrors the screen"));
    assert!(warns("tprov2").contains("mirrors the screen"));
    assert!(warns("tlite").contains("no T-Lite F4 firmware"));
    let tlite = profile("tlite").unwrap();
    assert!(tlite.look_alikes.iter().any(|l| l.board.is_none()));
    assert!(warns("v14").contains("v14lcd"));
}

#[test]
fn warnings_name_the_unproven_the_fork_and_the_quirks() {
    let v15 = profile("v15").unwrap().warnings().join("\n");
    assert!(
        v15.contains("has not proven the HelloRadioSky V15"),
        "{v15}"
    );
    assert!(v15.contains("helloradiosky/edgetx"), "{v15}");
    assert!(v15.contains("UF2 file"), "{v15}");
    let t22 = profile("t22").unwrap().warnings().join("\n");
    assert!(
        t22.contains("Early boards cannot be flashed over ROM DFU"),
        "{t22}"
    );
    let gx12 = profile("gx12").unwrap().warnings();
    assert!(
        gx12.iter().all(|w| !w.contains("has not proven")),
        "{gx12:?}"
    );
    let i = info(profile("tx16s").unwrap(), BoardFrom::RadioYml);
    assert!(!i.proven);
    assert_eq!(i.dfu_steps.len(), 3);
    assert!(i.warnings[0].contains("does not flash or write it"));
}

#[test]
fn a_board_with_several_rf_variants_has_no_one_elrs_target() {
    let t20 = profile("t20").unwrap();
    assert_eq!(t20.elrs.len(), 2);
    assert!(t20.elrs_target().is_none());
    assert!(profile("tx12").unwrap().elrs.is_empty());
    assert!(profile("tx12").unwrap().elrs_target().is_none());
    for p in PROFILES {
        for e in p.elrs {
            assert_eq!(e.target.split('.').count(), 3, "{}", e.target);
            assert!(!e.upload_methods.is_empty());
        }
    }
}

#[test]
fn a_firmware_splash_board_takes_the_128x64_patch() {
    for p in PROFILES.iter().filter(|p| p.proven()) {
        if p.splash == SplashKind::FirmwareMono128x64 {
            assert!(
                super::super::splash::board_supported(p.board),
                "{}",
                p.board
            );
        }
    }
    assert_eq!(
        profile("x9d").unwrap().splash,
        SplashKind::FirmwareMono212x64
    );
    assert_eq!(profile("tx16s").unwrap().splash, SplashKind::StorageImage);
}

#[test]
fn the_card_engine_controls_sit_in_the_profile_slots() {
    use crate::gear::edgetx::board::BOARDS;
    for c in BOARDS {
        let p = profile(c.board).unwrap();
        let kind = |n: &str| p.slots.iter().find(|s| s.name == n).map(|s| s.kind);
        for (n, _) in c.switches {
            assert_eq!(kind(n), Some(BoardControlKind::Switch), "{} {n}", c.board);
        }
        for n in c.custom {
            assert_eq!(kind(n), Some(BoardControlKind::Custom), "{} {n}", c.board);
        }
        for (n, _) in c.pots {
            assert_eq!(kind(n), Some(BoardControlKind::Pot), "{} {n}", c.board);
        }
        for n in c.sliders {
            assert_eq!(kind(n), Some(BoardControlKind::Slider), "{} {n}", c.board);
        }
    }
}
