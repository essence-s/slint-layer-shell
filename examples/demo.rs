use slint_layer_shell::{
    layer_properties::{LayerAnchor, LayerType, WindowConf},
    run_windows, windows,
};

slint::slint! {
    export component BarUi inherits Window {
        background: #101018f0;
        HorizontalLayout {
            padding-left: 12px;
            padding-right: 12px;
            Text {
                text: "slint-layer-shell · shared event loop";
                color: white;
                vertical-alignment: center;
            }
            Rectangle {}
            Text { text: "idle = 0% cpu"; color: gray; vertical-alignment: center; }
        }
    }

    export component PanelUi inherits Window {
        background: transparent;
        callback toggle-bar();
        Rectangle {
            background: #1e1e2eee;
            border-radius: 12px;
            VerticalLayout {
                padding: 20px;
                spacing: 12px;
                Text { text: "Panel flotante"; color: white; font-size: 16px; }
                Rectangle {
                    height: 44px;
                    border-radius: 8px;
                    background: ta.pressed ? #45475a : #313244;
                    ta := TouchArea {
                        clicked => {
                            box.x = box.x > 0 ? 0 : parent.width - box.width;
                        }
                    }
                    HorizontalLayout {
                        alignment: center;
                        Text { text: "Animar caja"; color: white; }
                    }
                }
                Rectangle {
                    height: 44px;
                    border-radius: 8px;
                    background: ta2.pressed ? #45475a : #313244;
                    ta2 := TouchArea {
                        clicked => { root.toggle-bar(); }
                    }
                    HorizontalLayout {
                        alignment: center;
                        Text { text: "Toggle barra"; color: white; }
                    }
                }
                box := Rectangle {
                    x: 0;
                    width: 36px;
                    height: 36px;
                    border-radius: 18px;
                    background: #f5c2e7;
                    animate x { duration: 800ms; easing: ease-in-out; }
                }
            }
        }
    }
}

windows!(BarUi, PanelUi);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::builder().filter_level(log::LevelFilter::Info).init();

    let bar_conf = WindowConf::builder()
        .width(1366_u32)
        .height(36_u32)
        .anchor_1(LayerAnchor::TOP | LayerAnchor::LEFT | LayerAnchor::RIGHT)
        .exclusive_zone(36)
        .layer_type(LayerType::Top)
        .build()
        .unwrap();

    let panel_conf = WindowConf::builder()
        .width(300_u32)
        .height(280_u32)
        .anchor_1(LayerAnchor::TOP | LayerAnchor::RIGHT)
        .margins(60, 20, 0, 0)
        .layer_type(LayerType::Overlay)
        .build()
        .unwrap();

    let bar = BarUiWl::spawn("demo-bar", bar_conf);
    let panel = PanelUiWl::spawn("demo-panel", panel_conf);

    // Toggle the bar from the panel: exercises WinHandle + hide/show path.
    let bar_handle = bar.get_handler();
    panel.on_toggle_bar(move || {
        bar_handle.toggle();
    });

    // Optional scripted check: DEMO_AUTOTEST=1 hides/shows the bar through
    // slint timers (also validates the TimerList -> calloop timeout path).
    if std::env::var_os("DEMO_AUTOTEST").is_some() {
        let h1 = bar.get_handler();
        let h2 = bar.get_handler();
        slint::Timer::single_shot(std::time::Duration::from_secs(2), move || {
            log::info!("AUTOTEST: hiding bar");
            h1.hide();
            slint::Timer::single_shot(std::time::Duration::from_secs(2), move || {
                log::info!("AUTOTEST: showing bar");
                h2.show_again();
            });
        });
    }

    run_windows![windows: [bar, panel]]
}
