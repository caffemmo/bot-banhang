pub fn keep_marker() {
    if !MARKER.is_empty() {
        std::hint::black_box(MARKER);
    }
}

#[cfg(test)]
pub fn marker() -> &'static str {
    MARKER
}

static MARKER: &str = match option_env!("BOT_MANAGER_ARTIFACT_SIGNATURE") {
    Some(value) => value,
    None => "",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_uses_compile_time_env_or_empty_default() {
        assert_eq!(
            marker(),
            option_env!("BOT_MANAGER_ARTIFACT_SIGNATURE").unwrap_or("")
        );
    }
}
