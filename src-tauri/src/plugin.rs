pub const PLUGIN_PROTOCOL: u32 = 1;
pub const PLUGIN_ID: &str = "multica";
pub const PIPE_NAME: &str = r"\\.\pipe\background-studio-multica";

pub fn is_plugin_mode() -> bool {
    std::env::args().any(|argument| argument == "--plugin")
}
