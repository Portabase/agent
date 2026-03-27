use crate::utils::task_manager::cron::next_run_timestamp;
use crate::utils::text::normalize_cron;
use cron::Schedule;
use std::str::FromStr;

#[test]
fn normalize_adds_seconds_to_five_field_cron() {
    let input = "*/5 * * * *";
    let normalized = normalize_cron(input);

    assert_eq!(normalized, "0 */5 * * * *");
}

#[test]
fn normalize_keeps_six_field_cron() {
    let input = "0 */5 * * * *";
    let normalized = normalize_cron(input);

    assert_eq!(normalized, "0 */5 * * * *");
}

#[test]
fn normalized_expression_is_valid_for_cron_schedule() {
    let input = "*/5 * * * *";
    let normalized = normalize_cron(input);

    let schedule = Schedule::from_str(&normalized);
    assert!(schedule.is_ok());
}

#[test]
fn next_run_timestamp_returns_future_timestamp() {
    let expr = normalize_cron("*/1 * * * *");
    let ts = next_run_timestamp(&expr);

    let now = chrono::Local::now().timestamp();
    assert!(ts > now);
}

#[test]
fn normalization_does_not_break_schedule_parsing() {
    let input = "0 */10 * * * *";
    let normalized = normalize_cron(input);

    let schedule = Schedule::from_str(&normalized).unwrap();
    let next = schedule.upcoming(chrono::Local).next();

    assert!(next.is_some());
}
