//! The `pp.native` bridge script.

use crate::bridge::*;

fn script() -> String {
    bridge_script("windows", "1.2.3")
}

#[test]
fn the_script_carries_the_allowlist_it_will_enforce() {
    let js = script();
    for command in crate::capabilities::commands() {
        assert!(
            js.contains(&format!("\"{}\"", command.name)),
            "`{}` is missing from the bridge",
            command.name
        );
        assert!(
            js.contains(&format!("\"{}\"", command.capability)),
            "capability `{}` is missing from the bridge",
            command.capability
        );
    }
}

#[test]
fn the_script_reports_the_platform_and_version_it_was_built_for() {
    let js = script();
    assert!(js.contains("platform: \"windows\""), "{js}");
    assert!(js.contains("version: \"1.2.3\""), "{js}");
    assert!(js.contains("available: true"));
}

#[test]
fn a_value_cannot_close_the_script_it_is_written_into() {
    // The same rule `rahti::Json` follows: this text is inside a `<script>`,
    // and `</script>` in a value would end it.
    let js = bridge_script("</script><script>alert(1)</script>", "1.0.0");
    assert!(!js.contains("</script>"), "{js}");
    assert!(js.contains("\\u003c/script"), "{js}");
}

#[test]
fn the_script_does_not_reexport_the_tauri_surface() {
    let js = script();
    // A page must not be able to reach a plugin command the application never
    // meant to expose merely because the plugin was linked in.
    assert!(!js.contains("window.__TAURI__.core.invoke.bind"));
    assert!(js.contains("ALLOWED.indexOf(name) === -1"));
}

#[test]
fn the_script_attaches_to_pulsepoint_without_editing_it() {
    let js = script();
    // The bundle publishes `globalThis.pp` when it loads, which is after this
    // script runs. Intercepting the assignment is what adds the namespace
    // without a byte of the minified runtime changing.
    assert!(js.contains("Object.defineProperty(window, \"pp\""), "{js}");
    // The documented access point is a global. A reactive block's `pp` is an
    // object PulsePoint supplies to the function it compiles, so `pp.native`
    // is not reachable there and `window.rahtiNative` is.
    assert!(js.contains("window.rahtiNative = api"), "{js}");
}

#[test]
fn blank_links_work_without_a_second_webview() {
    let js = script();
    assert!(
        js.contains("String(anchor.target).toLowerCase() !== \"_blank\""),
        "{js}"
    );
    assert!(js.contains("url.origin === window.location.origin"), "{js}");
    assert!(
        js.contains("api.invoke(\"open_external\", { url: url.href })"),
        "{js}"
    );
}

#[test]
fn only_pages_using_popovers_warn_about_an_old_webview() {
    let js = script();
    assert!(js.contains("HTMLElement.prototype.showPopover"), "{js}");
    assert!(js.contains("document.querySelector(\"[popover]\")"), "{js}");
    assert!(js.contains("rahti-webview-compatibility"), "{js}");
}

#[tokio::test]
async fn the_route_serves_it_as_javascript_and_never_from_a_cache() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let response = bridge_route("android", "0.4.0".to_string())
        .oneshot(
            Request::builder()
                .uri(BRIDGE_PATH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("a response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    // The allowlist is compiled in, so a cached copy would outlive an upgrade
    // that removed a command.
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let js = String::from_utf8_lossy(&body);
    assert!(js.contains("platform: \"android\""));
    assert!(js.contains("version: \"0.4.0\""));
}
