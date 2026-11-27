fn app_context() -> tauri::Context {
    tauri::generate_context!()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(app_context())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::app_context;

    #[test]
    fn product_name_is_shikigami() {
        assert_eq!(
            app_context().config().product_name.as_deref(),
            Some("Shikigami")
        );
    }
}
