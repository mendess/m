pub fn with_video_env() -> bool {
    let Some(var) = std::env::var_os("WITH_VIDEO") else {
        return false;
    };
    let Some(var) = var.to_str() else {
        return false;
    };
    var == "1" || var == "true"
}
