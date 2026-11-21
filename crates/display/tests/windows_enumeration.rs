#![cfg(target_os = "windows")]

use std::collections::HashSet;

use wallpaper_display::enumerate_displays;

#[test]
fn enumerates_active_displays_with_stable_paths_and_physical_bounds() {
    let snapshot = enumerate_displays().expect("active display enumeration should succeed");
    let repeated = enumerate_displays().expect("a repeated enumeration should succeed");
    assert_eq!(snapshot, repeated, "stable IDs and geometry should repeat");
    assert!(
        !snapshot.displays().is_empty(),
        "the Windows desktop should expose at least one active display"
    );

    let mut ids = HashSet::new();
    for display in snapshot.displays() {
        assert!(!display.id().as_str().is_empty());
        assert!(ids.insert(display.id().as_str()));
        assert!(!display.name().is_empty());
        assert!(display.bounds().width() > 0);
        assert!(display.bounds().height() > 0);
        assert!((50..=800).contains(&display.dpi_scale_percent()));
        println!(
            "display id={:?} name={:?} device={:?} bounds=({}, {}, {}, {}) resolution={}x{} scale={} effective-dpi={} refresh={:?} primary={}",
            display.id().as_str(),
            display.name(),
            display.device_name(),
            display.bounds().left(),
            display.bounds().top(),
            display.bounds().right(),
            display.bounds().bottom(),
            display.resolution().width(),
            display.resolution().height(),
            display.dpi_scale_percent(),
            display.effective_dpi(),
            display.refresh_rate(),
            display.is_primary()
        );
    }
    println!("active_display_count={}", snapshot.displays().len());
}
