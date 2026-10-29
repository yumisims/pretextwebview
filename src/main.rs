mod app;
mod colormap;
mod model;
mod pretext;
mod suggest;
mod yumi;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_title("pretextwebview"),
        ..Default::default()
    };
    eframe::run_native(
        "pretextwebview",
        options,
        Box::new(|cc| Ok(Box::new(app::PretextApp::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast;

    console_error_panic_hook::set_once();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        if let Some(loading) = eframe::web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("loading_text"))
        {
            loading.set_inner_html(&format!("pretextwebview panicked: {info}"));
        }
    }));

    let web_options = eframe::WebOptions::default();
    wasm_bindgen_futures::spawn_local(async {
        let document = eframe::web_sys::window()
            .expect("no window")
            .document()
            .expect("no document");
        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("missing the_canvas_id")
            .dyn_into::<eframe::web_sys::HtmlCanvasElement>()
            .expect("the_canvas_id is not a canvas");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(app::PretextApp::new(cc)))),
            )
            .await;

        if let Some(loading) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(()) => {
                    loading.remove();
                }
                Err(err) => {
                    loading.set_inner_html(&format!("pretextwebview failed to start: {err:?}"));
                }
            }
        }
    });
}
