//! The S1 cost bar in CI: p99 step cost under 100 µs at 2 kHz, in free flight and in
//! contact, and contact no more than twice a free-flight step (sim-design 2.2, 4.7).

use quadcam_sim::bench;

#[test]
fn step_cost_p99_is_under_100_us_free_and_in_contact() {
    for id in ["meteor75", "five_inch"] {
        let free = bench::free_flight(id, 40_000);
        let (contact, frac) = bench::contact(id, 40_000);
        let (f99, c99) = (free.quantile_us(0.99), contact.quantile_us(0.99));
        println!(
            "{id}: free p99 {f99} us, contact p99 {c99} us ({:.0} % touching)",
            frac * 100.0
        );
        assert!(
            frac > 0.5,
            "{id}: the contact run touched only {:.0} %",
            frac * 100.0
        );
        assert!(f99 < 100.0 && c99 < 100.0, "{id}: p99 {f99} / {c99} us");
        assert!(
            contact.quantile_us(0.5) <= 2.0 * free.quantile_us(0.5).max(1.0),
            "{id}: contact median {} vs free {}",
            contact.quantile_us(0.5),
            free.quantile_us(0.5)
        );
    }
}
