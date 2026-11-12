pub fn retry_allowed(attempts: u32) -> bool {
    attempts < 3
}
