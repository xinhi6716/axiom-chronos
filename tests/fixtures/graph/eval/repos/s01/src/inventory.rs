pub fn reserve(available: u32, requested: u32) -> Option<u32> {
    if requested > available { None } else { Some(available - requested) }
}
