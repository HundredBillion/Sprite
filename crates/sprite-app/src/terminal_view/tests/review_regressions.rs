use super::*;

fn with_view(
    cx: &mut gpui::TestAppContext,
    command: Vec<std::ffi::OsString>,
    run: impl FnOnce(&gpui::Entity<TerminalView>, &mut gpui::VisualTestContext),
) {
    let settings = crate::config::Settings::default();
    cx.set_global(crate::config::ActiveSettings(settings.clone()));
    cx.set_global(crate::tokens::TokenRegistry::new(&settings.colors));
    let (sender, _exits) = async_channel::unbounded();
    let (view, cx) = cx.add_window_view(|window, cx| {
        TerminalView::new(
            Some(command),
            settings,
            Vec::new(),
            None,
            PaneExit {
                sender,
                identity: (crate::tabs::TabId(1), crate::pane_tree::PaneId(1)),
            },
            window,
            cx,
        )
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_bundle(&view, cx, |_| true);
        run(&view, cx);
    }));
    if let Some(cleanup) = view.update(cx, |view, _| view.begin_shutdown()) {
        cleanup.wait().unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

fn shell(script: &str) -> Vec<std::ffi::OsString> {
    vec!["/bin/sh".into(), "-c".into(), script.into()]
}

#[gpui::test]
fn review_direct_program_requires_close_confirmation(cx: &mut gpui::TestAppContext) {
    with_view(cx, vec!["/bin/cat".into()], |view, cx| {
        view.update(cx, |view, _| {
            view.send(TerminalCommand::Input(b"READY\n".to_vec()))
        });
        wait_for_bundle(view, cx, |bundle| {
            bundle
                .pane
                .rows
                .iter()
                .any(|row| row.text.contains("READY"))
        });
        assert!(
            view.read_with(cx, |view, _| sprite_pane::Pane::close_warning(view)
                .is_some()),
            "a directly launched foreground program must require close confirmation"
        );
    });
}

/// An image placed below the text (a negative z-index) is drawn between the
/// cells' grounds and their glyphs, so text printed over it stays readable.
/// The grid's two passes are each one layer, and this is the order those two
/// layers and the image between them reach the renderer in.
#[gpui::test]
fn an_image_below_the_text_is_drawn_under_the_glyphs_over_it(cx: &mut gpui::TestAppContext) {
    use gpui::DrawnKind;
    gpui::ink_test_glyphs(true);
    with_view(
        cx,
        shell(
            r"printf '\033_Ga=T,f=32,s=1,v=1,i=1,c=4,r=1,z=-1,C=1,q=2;/////w==\033\\WWWW'; exec sleep 30",
        ),
        |view, cx| {
            wait_for_bundle(view, cx, |bundle| {
                bundle
                    .graphics
                    .as_ref()
                    .is_some_and(|frame| !frame.placements.is_empty())
                    && bundle
                        .render
                        .rows
                        .iter()
                        .any(|row| row.cells.iter().any(|cell| cell.text == "W"))
            });
            focus_and_draw(view, cx);
            let drawn = cx.drawn_primitives();
            let image = drawn
                .iter()
                .position(|primitive| primitive.kind == DrawnKind::PolychromeSprite)
                .unwrap_or_else(|| panic!("the image is drawn: {drawn:?}"));
            let over: Vec<usize> = (0..drawn.len())
                .filter(|&index| {
                    drawn[index].kind == DrawnKind::MonochromeSprite
                        && drawn[index].bounds.intersects(&drawn[image].bounds)
                })
                .collect();
            assert!(!over.is_empty(), "letters lie over the image: {drawn:?}");
            assert!(
                over.iter().all(|&glyph| glyph > image),
                "every letter over the image is drawn after it: {drawn:?}"
            );
        },
    );
}

#[gpui::test]
fn review_fixed_image_matches_requested_columns_at_two_x(cx: &mut gpui::TestAppContext) {
    with_view(
        cx,
        shell(r"printf '\033_Ga=T,f=32,s=1,v=1,i=1,c=10,r=2,q=2;/////w==\033\\'; exec sleep 30"),
        |view, cx| {
            cx.simulate_scale_factor(2.0);
            wait_for_bundle(view, cx, |bundle| {
                bundle
                    .graphics
                    .as_ref()
                    .is_some_and(|frame| !frame.placements.is_empty())
            });
            focus_and_draw(view, cx);
            let scale = cx.update(|window, _| window.scale_factor());
            assert_eq!(scale, 2.0);
            let expected = view.read_with(cx, |view, _| {
                let placement = &view
                    .bundle
                    .as_ref()
                    .unwrap()
                    .graphics
                    .as_ref()
                    .unwrap()
                    .placements[0];
                px(placement.pixel_width as f32 / scale)
            });
            let drawn = cx
                .debug_bounds("review-fixed-image")
                .expect("fixed image was laid out");
            eprintln!(
                "requested 10 columns: expected={expected:?}, drawn={:?}",
                drawn.size.width
            );
            assert!(
                (f32::from(drawn.size.width) - f32::from(expected)).abs() < 1.0,
                "physical placement dimensions must be converted to logical pixels"
            );
        },
    );
}

#[gpui::test]
fn fixed_image_crop_and_offsets_use_logical_pixels_at_each_scale(cx: &mut gpui::TestAppContext) {
    let script = format!(
        "printf '\\033_Ga=T,f=32,s=8,v=6,i=11,c=10,r=2,x=2,y=1,w=4,h=3,X=4,Y=6,q=2;{}\\033\\\\'; exec sleep 30",
        "/".repeat(256)
    );
    for scale in [1.0, 2.0] {
        with_view(cx, shell(&script), |view, cx| {
            cx.simulate_scale_factor(scale);
            wait_for_bundle(view, cx, |bundle| {
                bundle
                    .graphics
                    .as_ref()
                    .is_some_and(|frame| !frame.placements.is_empty())
            });
            focus_and_draw(view, cx);
            let placement = view.read_with(cx, |view, _| {
                view.bundle
                    .as_ref()
                    .unwrap()
                    .graphics
                    .as_ref()
                    .unwrap()
                    .placements[0]
            });
            assert_eq!(
                (
                    placement.source.x,
                    placement.source.y,
                    placement.source.width,
                    placement.source.height
                ),
                (2, 1, 4, 3)
            );
            assert_eq!((placement.x_offset, placement.y_offset), (4, 6));
            let origin = view
                .read_with(cx, |view, _| view.content_origin_for_test())
                .unwrap();
            let metrics = view.read_with(cx, |view, _| view.terminal_grid_metrics());
            let outer = cx.debug_bounds("review-fixed-image").unwrap();
            let inner = cx.debug_bounds("review-fixed-image-content").unwrap();
            let width = px(placement.pixel_width as f32 / scale);
            let height = px(placement.pixel_height as f32 / scale);
            let expected_origin = gpui::point(
                origin.x
                    + metrics.cells.width() * placement.viewport_column as f32
                    + px(4.0 / scale),
                origin.y + metrics.cells.height() * placement.viewport_row as f32 + px(6.0 / scale),
            );
            assert_eq!(outer.origin, expected_origin);
            assert_eq!(outer.size, gpui::size(width, height));
            let crop_origin = outer.origin - gpui::point(width * 0.5, height / 3.0);
            assert!((inner.origin.x - crop_origin.x).abs() <= px(1.0 / scale));
            assert!((inner.origin.y - crop_origin.y).abs() <= px(1.0 / scale));
            assert_eq!(inner.size, gpui::size(width * 2.0, height * 2.0));
        });
    }
}

#[gpui::test]
fn review_ime_stays_at_cursor_across_blink(cx: &mut gpui::TestAppContext) {
    with_view(
        cx,
        shell(r"printf '\033[5;9H\033[?12h'; exec sleep 30"),
        |view, cx| {
            wait_for_bundle(view, cx, |bundle| {
                bundle.render.cursor.row == 4
                    && bundle.render.cursor.column == 8
                    && bundle.render.cursor.blinking
            });
            focus_and_draw(view, cx);
            view.update_in(cx, |view, window, cx| {
                gpui::EntityInputHandler::replace_and_mark_text_in_range(
                    view, None, "日本", None, window, cx,
                );
                view.blink_on = true;
            });
            redraw(cx);
            let visible = cx
                .debug_bounds("review-preedit")
                .expect("composition is drawn");
            clock_beat(view, cx);
            assert!(!view.read_with(cx, |view, _| view.blink_on));
            redraw(cx);
            let hidden = cx
                .debug_bounds("review-preedit")
                .expect("composition remains drawn");
            eprintln!(
                "composition blink on={:?}, off={:?}",
                visible.origin, hidden.origin
            );
            assert_eq!(
                visible.origin, hidden.origin,
                "blinking must not move composition text"
            );
        },
    );
}

#[gpui::test]
fn review_unchanged_images_reuse_textures_under_collective_pressure(cx: &mut gpui::TestAppContext) {
    let script = r#"stty -echo; for id in 1 2 3; do printf '\033_Ga=T,f=32,s=1,v=1,i=%s,q=2;/////w==\033\\' "$id"; done; printf READY; while IFS= read -r line; do printf UPDATED; done"#;
    with_view(cx, shell(script), |view, cx| {
        let initial = wait_for_bundle(view, cx, |bundle| {
            bundle
                .graphics
                .as_ref()
                .is_some_and(|frame| frame.images.len() == 3)
                && bundle
                    .pane
                    .rows
                    .iter()
                    .any(|row| row.text.contains("READY"))
        });
        let before = view.update(cx, |view, _| {
            view.textures.set_budget(8);
            view.refresh_textures(&initial);
            initial
                .graphics
                .as_ref()
                .unwrap()
                .images
                .iter()
                .filter_map(|image| {
                    view.textures
                        .get(image.id, image.generation)
                        .map(|texture| (image.id, image.generation, texture))
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(before.len(), 2);
        view.update(cx, |view, _| {
            view.send(TerminalCommand::Input(b"go\n".to_vec()))
        });
        let next = wait_for_bundle(view, cx, |bundle| {
            bundle
                .pane
                .rows
                .iter()
                .any(|row| row.text.contains("UPDATED"))
        });
        for (a, b) in initial
            .graphics
            .as_ref()
            .unwrap()
            .images
            .iter()
            .zip(&next.graphics.as_ref().unwrap().images)
        {
            assert_eq!((a.id, a.generation), (b.id, b.generation));
        }
        let rebuilt = view.read_with(cx, |view, _| {
            before
                .iter()
                .filter(|(id, generation, texture)| {
                    view.textures
                        .get(*id, *generation)
                        .is_none_or(|after| !Arc::ptr_eq(texture, &after))
                })
                .count()
        });
        eprintln!(
            "unchanged cached textures replaced after text-only output: {rebuilt}/{}",
            before.len()
        );
        assert_eq!(
            rebuilt, 0,
            "text-only output must not rebuild retained unchanged images"
        );
    });
}
