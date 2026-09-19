#[cfg(target_os = "linux")]
pub fn install(cx: &mut gpui::App) {
    cx.text_system()
        .add_fonts(vec![
            std::borrow::Cow::Borrowed(REGULAR),
            std::borrow::Cow::Borrowed(BOLD),
        ])
        .expect("load bundled Adwaita Sans faces");
}

#[cfg(target_os = "linux")]
const REGULAR: &[u8] = include_bytes!("../assets/fonts/AdwaitaSans-Regular.ttf");
#[cfg(target_os = "linux")]
const BOLD: &[u8] = include_bytes!("../assets/fonts/AdwaitaSans-Bold.ttf");

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use resvg::usvg::fontdb::{Database, Family, Query, Weight};

    #[test]
    fn bundled_faces_select_distinct_regular_and_bold_weights() {
        let mut db = Database::new();
        db.load_font_data(REGULAR.to_vec());
        db.load_font_data(BOLD.to_vec());
        let families = [Family::Name("Adwaita Sans")];
        let regular = db.query(&Query {
            families: &families,
            weight: Weight::NORMAL,
            ..Query::default()
        });
        let bold = db.query(&Query {
            families: &families,
            weight: Weight::BOLD,
            ..Query::default()
        });
        assert_ne!(regular, bold);
        assert_eq!(db.face(regular.unwrap()).unwrap().weight, Weight::NORMAL);
        assert_eq!(db.face(bold.unwrap()).unwrap().weight, Weight::BOLD);
    }
}
