//! The Android launch screen.
//!
//! ## The bug this exists for
//!
//! Android draws a window the moment the launcher is tapped, long before a
//! WebView has a page. Tauri's generated project leaves that window to the
//! Material theme, so a launch is a white rectangle (black in dark mode),
//! then the WebView's own blank white, then the page. Android 12 and later do
//! put the launcher icon on it, and take it down at the activity's first
//! frame — which is still before the page.
//!
//! So every application wrote the same launch screen by hand, in Kotlin, in a
//! directory that is regenerated.
//!
//! ## What is written
//!
//! - Resources: the two colours, a theme for the launcher activity, and for
//!   Android before 12 a window background with the icon on it.
//! - `RahtiLaunchScreen.kt`, beside `MainActivity.kt`, which keeps the splash
//!   on top until the WebView reports the page ready to draw.
//! - One attribute on the launcher activity and one line in its `onCreate`.
//!
//! The files are this tool's and are rewritten; the two edits are idempotent
//! and refuse a shape they do not recognise. `enabled: false` takes all of it
//! out again, so a shell with its own launch screen is not left with two.
//!
//! Applied on every Android build, like the other patches in `run`:
//! `gen/android` is regenerated rather than reviewed.

use std::path::Path;

use rahti_native::{NativeError, SplashConfig};

/// Marks the line this tool adds to `MainActivity.kt`.
const MARKER: &str = "rahti-native: hold the launch screen";

/// What goes into `onCreate`, after `super.onCreate`.
const CODE: &str = "    // rahti-native: hold the launch screen until the first page has painted.
    RahtiLaunchScreen.install(this)
";

/// The activity theme the launch screen is.
const THEME: &str = "Theme.Rahti.Launch";

/// The attribute Tauri writes on the launcher activity, and the anchor.
const ACTIVITY_ANCHOR: &str = "android:name=\".MainActivity\"";

/// What the manifest gains, ahead of the anchor.
const ACTIVITY_THEME: &str = "android:theme=\"@style/Theme.Rahti.Launch\"\n            ";

/// The resource files this tool owns, under `app/src/main/res`.
pub(crate) const RESOURCES: &[&str] = &[
    "values/rahti_launch.xml",
    "values-night/rahti_launch.xml",
    "values-v31/rahti_launch.xml",
    "drawable/rahti_launch_background.xml",
];

/// Beside `MainActivity.kt`, in its package.
pub(crate) const CLASS_FILE: &str = "RahtiLaunchScreen.kt";

/// Show the application's icon on its own colour until the first page is on
/// screen, and fade once. Or, when the configuration says not to, take a
/// launch screen an earlier build installed out again.
pub(crate) fn install(generated: &Path, splash: &SplashConfig) -> Result<(), NativeError> {
    let res = generated.join("app/src/main/res");
    let manifest_path = generated.join("app/src/main/AndroidManifest.xml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .map_err(|e| NativeError::io("android", &manifest_path, e))?;
    let activity_path = crate::run::find_main_activity(&generated.join("app/src/main/java"));

    if !splash.enabled {
        let mut removed = false;
        for file in RESOURCES {
            removed |= std::fs::remove_file(res.join(file)).is_ok();
        }
        if let Some(path) = &activity_path {
            removed |= std::fs::remove_file(path.with_file_name(CLASS_FILE)).is_ok();
            let source =
                std::fs::read_to_string(path).map_err(|e| NativeError::io("android", path, e))?;
            if let Some(patched) = with_launch_screen(&source, false) {
                removed |= write_if_changed(path, &patched)?;
            }
        }
        removed |= write_if_changed(&manifest_path, &manifest.replace(ACTIVITY_THEME, ""))?;
        if removed {
            println!("  android: removed the launch screen (android.splash.enabled is false)");
        }
        return Ok(());
    }

    let refuse = |path: &Path, what: &str| {
        NativeError::at(
            "android",
            path,
            format!(
                "{what}, so the launch screen was not installed.\n  \
                 Delete native/gen/android to generate it again, or set \
                 `android.splash.enabled` to false in rahti.native.json if this shell has its \
                 own."
            ),
        )
    };

    let Some(activity_path) = activity_path else {
        return Err(refuse(
            generated,
            "this Android project has no MainActivity.kt",
        ));
    };
    let source = std::fs::read_to_string(&activity_path)
        .map_err(|e| NativeError::io("android", &activity_path, e))?;

    // Everything is worked out before anything is written, so a project this
    // tool does not recognise is left exactly as it was.
    let patched_manifest = with_launch_theme(&manifest).ok_or_else(|| {
        refuse(
            &manifest_path,
            "this Android manifest is not one `cargo rahti native` recognises",
        )
    })?;
    let patched_activity = with_launch_screen(&source, true).ok_or_else(|| {
        refuse(
            &activity_path,
            "this MainActivity is not one `cargo rahti native` recognises",
        )
    })?;
    let Some(package) = kotlin_package(&source) else {
        return Err(refuse(&activity_path, "this MainActivity names no package"));
    };

    let icon = launch_icon(&res);
    let mut changed = false;
    for (file, content) in resources(splash, &application_theme(&manifest), icon) {
        let path = res.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| NativeError::io("android", parent, e))?;
        }
        changed |= write_if_changed(&path, &content)?;
    }
    changed |= write_if_changed(&activity_path.with_file_name(CLASS_FILE), &class(package))?;
    changed |= write_if_changed(&manifest_path, &patched_manifest)?;
    changed |= write_if_changed(&activity_path, &patched_activity)?;

    if changed {
        println!(
            "  android: launch screen on {} ({} in dark mode)",
            splash.background, splash.background_dark
        );
    }
    Ok(())
}

/// Write `content` unless the file already holds it. Whether it wrote.
pub(crate) fn write_if_changed(path: &Path, content: &str) -> Result<bool, NativeError> {
    if std::fs::read_to_string(path).is_ok_and(|current| current == content) {
        return Ok(false);
    }
    std::fs::write(path, content).map_err(|e| NativeError::io("android", path, e))?;
    Ok(true)
}

/// The manifest with the launch theme on the launcher activity, or `None` for
/// a manifest whose activity this tool does not recognise. Idempotent.
pub(crate) fn with_launch_theme(manifest: &str) -> Option<String> {
    if manifest.contains(THEME) {
        return Some(manifest.to_string());
    }
    if manifest.matches(ACTIVITY_ANCHOR).count() != 1 {
        return None;
    }
    // An activity that already names a theme has a launch screen of its own,
    // and a second one is not this tool's to add.
    let at = manifest.find(ACTIVITY_ANCHOR)?;
    let open = manifest[..at].rfind("<activity")?;
    let close = at + manifest[at..].find('>')?;
    if manifest[open..close].contains("android:theme=") {
        return None;
    }
    Some(manifest.replacen(
        ACTIVITY_ANCHOR,
        &format!("{ACTIVITY_THEME}{ACTIVITY_ANCHOR}"),
        1,
    ))
}

/// The theme Tauri gave the application, which the launch theme extends so
/// that everything else about the activity stays as generated.
pub(crate) fn application_theme(manifest: &str) -> String {
    const OPEN: &str = "android:theme=\"@style/";
    manifest
        .match_indices(OPEN)
        .filter_map(|(at, _)| manifest[at + OPEN.len()..].split('"').next())
        .find(|name| {
            *name != THEME
                && !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
        })
        .unwrap_or("Theme.MaterialComponents.DayNight.NoActionBar")
        .to_string()
}

/// `MainActivity.kt` with the launch screen's line in it (or, for `false`,
/// without it), or `None` for a shape this tool does not recognise.
pub(crate) fn with_launch_screen(source: &str, enabled: bool) -> Option<String> {
    with_on_create(source, MARKER, CODE, enabled)
}

/// `MainActivity.kt` with `code` after `super.onCreate` (or, for `false`,
/// without it), or `None` for a shape this tool does not recognise.
///
/// `marker` is text inside `code` that says it is already there, which is
/// what makes this idempotent.
pub(crate) fn with_on_create(
    source: &str,
    marker: &str,
    code: &str,
    enabled: bool,
) -> Option<String> {
    if source.contains(marker) == enabled {
        return Some(source.to_string());
    }
    let crlf = source.contains("\r\n");
    let text = source.replace("\r\n", "\n");

    const SUPER: &str = "super.onCreate(savedInstanceState)\n";
    let patched = if !enabled {
        text.replace(code, "")
    } else if text.matches(SUPER).count() == 1 {
        text.replacen(SUPER, &format!("{SUPER}{code}"), 1)
    } else {
        return None;
    };

    Some(if crlf {
        patched.replace('\n', "\r\n")
    } else {
        patched
    })
}

/// The package a Kotlin source declares.
pub(crate) fn kotlin_package(source: &str) -> Option<&str> {
    let package = source
        .lines()
        .find_map(|line| line.trim().strip_prefix("package "))?
        .trim()
        .trim_end_matches(';');
    let legal = !package.is_empty()
        && package
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
    legal.then_some(package)
}

/// The resource the launch screen draws: the application's artwork inside
/// the adaptive icon's safe zone, drawn large by `cargo rahti native icon` and
/// copied in with the launcher icons.
///
/// `None` when the project has no icon tree, and the launch screen is then
/// the colour alone before Android 12 and the system's choice of icon after.
fn launch_icon(res: &Path) -> Option<&'static str> {
    res.join("drawable-nodpi/rahti_launch_icon.png")
        .is_file()
        .then_some("@drawable/rahti_launch_icon")
}

/// The launch screen's resource files, as `(path under res, content)`.
pub(crate) fn resources(
    splash: &SplashConfig,
    parent: &str,
    icon: Option<&str>,
) -> Vec<(&'static str, String)> {
    const HEADER: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
        <!-- Written by `cargo rahti native` from `android.splash` in rahti.native.json.\n     \
        Do not edit: it is rewritten on every Android build. -->\n";

    // Before Android 12 there is no splash screen, and the window background
    // is what a launch shows.
    let values = format!(
        "{HEADER}<resources>\n    \
         <color name=\"rahti_launch_background\">{}</color>\n    \
         <style name=\"{THEME}\" parent=\"{parent}\">\n        \
         <item name=\"android:windowBackground\">@drawable/rahti_launch_background</item>\n    \
         </style>\n\
         </resources>\n",
        splash.background
    );

    let night = format!(
        "{HEADER}<resources>\n    \
         <color name=\"rahti_launch_background\">{}</color>\n\
         </resources>\n",
        splash.background_dark
    );

    // Android 12 and later draw the splash themselves, from these.
    let animated = icon
        .map(|icon| {
            format!("        <item name=\"android:windowSplashScreenAnimatedIcon\">{icon}</item>\n")
        })
        .unwrap_or_default();
    let v31 = format!(
        "{HEADER}<resources>\n    \
         <style name=\"{THEME}\" parent=\"{parent}\">\n        \
         <item name=\"android:windowBackground\">@color/rahti_launch_background</item>\n        \
         <item name=\"android:windowSplashScreenBackground\">@color/rahti_launch_background</item>\n\
         {animated}    \
         </style>\n\
         </resources>\n"
    );

    // The box Android 12 gives an icon with no background of its own, so the
    // artwork is the same size on either side of that release.
    let layer = icon
        .map(|icon| {
            format!(
                "    <item\n        \
                 android:width=\"288dp\"\n        \
                 android:height=\"288dp\"\n        \
                 android:gravity=\"center\"\n        \
                 android:drawable=\"{icon}\" />\n"
            )
        })
        .unwrap_or_default();
    let background = format!(
        "{HEADER}<layer-list xmlns:android=\"http://schemas.android.com/apk/res/android\">\n    \
         <item android:drawable=\"@color/rahti_launch_background\" />\n\
         {layer}\
         </layer-list>\n"
    );

    vec![
        (RESOURCES[0], values),
        (RESOURCES[1], night),
        (RESOURCES[2], v31),
        (RESOURCES[3], background),
    ]
}

/// `RahtiLaunchScreen.kt`, in the application's package.
///
/// Resources are looked up by name rather than through `R`, so the file does
/// not depend on which package the generated `R` class is in.
pub(crate) fn class(package: &str) -> String {
    format!("package {package}\n{CLASS_BODY}")
}

const CLASS_BODY: &str = r#"
// Written by `cargo rahti native` on every Android build. Do not edit: set
// `android.splash` in rahti.native.json instead.

import android.app.Activity
import android.graphics.drawable.ColorDrawable
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.core.content.ContextCompat
import androidx.core.graphics.ColorUtils
import androidx.core.view.WindowCompat

/**
 * The launch screen: shown once, faded once, straight onto the first page.
 *
 * Android 12 and later show the theme's splash the moment the application
 * starts and take it down at the activity's first frame, which is before the
 * WebView has a page. So the splash is claimed with an exit listener, kept on
 * top while the page renders underneath, and faded when the WebView reports
 * it ready to draw. Before Android 12 the theme's window background plays
 * the splash, and the same drawable in a view on top takes its place.
 *
 * Holding the first frame back instead (a pre-draw listener) also stops the
 * WebView from painting, and the splash then fades onto a blank screen.
 */
internal class RahtiLaunchScreen private constructor(private val activity: Activity) {
  private val handler = Handler(Looper.getMainLooper())
  private val started = SystemClock.uptimeMillis()
  private var webView: WebView? = null
  private var waitingForPaint = false
  private var revealed = false

  /** What is on top; null until Android 12+ hands its splash over. */
  private var cover: View? = null
  private var removeCover: (() -> Unit)? = null

  private fun resource(name: String, type: String): Int =
    activity.resources.getIdentifier(name, type, activity.packageName)

  private val background: Int? =
    resource("rahti_launch_background", "color").takeIf { it != 0 }?.let {
      ContextCompat.getColor(activity, it)
    }

  /** Status and navigation bar icons that can be read on the launch colour. */
  private fun contrastSystemBars() {
    val color = background ?: return
    val light = ColorUtils.calculateLuminance(color) > 0.5
    WindowCompat.getInsetsController(activity.window, activity.window.decorView).apply {
      isAppearanceLightStatusBars = light
      isAppearanceLightNavigationBars = light
    }
  }

  private fun claim() {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      activity.splashScreen.setOnExitAnimationListener { view ->
        cover = view
        removeCover = { view.remove() }
        if (revealed) fade()
      }
      return
    }
    val drawable = resource("rahti_launch_background", "drawable")
    if (drawable == 0) return
    val overlay = View(activity).apply {
      setBackgroundResource(drawable)
      isClickable = true // nothing under the cover takes a tap
    }
    (activity.window.decorView as ViewGroup).addView(
      overlay,
      ViewGroup.LayoutParams(
        ViewGroup.LayoutParams.MATCH_PARENT,
        ViewGroup.LayoutParams.MATCH_PARENT
      )
    )
    cover = overlay
    removeCover = { (overlay.parent as? ViewGroup)?.removeView(overlay) }
  }

  private fun findWebView(view: View?): WebView? {
    if (view is WebView) return view
    if (view is ViewGroup) {
      for (index in 0 until view.childCount) {
        findWebView(view.getChildAt(index))?.let { return it }
      }
    }
    return null
  }

  private fun check() {
    if (revealed) return
    val view = webView
      ?: findWebView(activity.findViewById(android.R.id.content))?.also { webView = it }
    val url = view?.url
    when {
      // Whatever happened, the application is not left behind its splash.
      SystemClock.uptimeMillis() - started >= MAX_HOLD_MS -> reveal()
      view == null || url == null || url == "about:blank" -> Unit
      // The bundled start page of a hosted application: it is on its way
      // somewhere, so the splash stays unless it says it cannot get there.
      Uri.parse(url).host == START_PAGE_HOST ->
        if (Uri.parse(url).fragment == "offline") reveal()
      view.progress >= 100 && !waitingForPaint -> {
        waitingForPaint = true
        view.postVisualStateCallback(0, object : WebView.VisualStateCallback() {
          override fun onComplete(requestId: Long) {
            // Ready for the next draw; a frame or two to reach the screen.
            handler.postDelayed(::reveal, PAINT_SETTLE_MS)
          }
        })
      }
    }
    if (!revealed) handler.postDelayed(::check, POLL_MS)
  }

  private fun reveal() {
    if (revealed) return
    revealed = true
    // The page is kept clear of the system bars, so the window shows behind
    // them: the colour stays there, the icon does not.
    background?.let { activity.window.setBackgroundDrawable(ColorDrawable(it)) }
    fade()
  }

  private fun fade() {
    val view = cover ?: return
    cover = null
    view.animate()
      .alpha(0f)
      .setDuration(FADE_MS)
      .withEndAction { removeCover?.invoke() }
      .start()
  }

  companion object {
    private const val MAX_HOLD_MS = 10_000L
    private const val POLL_MS = 50L
    private const val PAINT_SETTLE_MS = 120L
    private const val FADE_MS = 300L

    /** The origin Tauri serves `dist/` from on Android. */
    private const val START_PAGE_HOST = "tauri.localhost"

    /** From `MainActivity.onCreate`, before the first frame. */
    fun install(activity: Activity) {
      RahtiLaunchScreen(activity).apply {
        contrastSystemBars()
        claim()
        check()
      }
    }
  }
}
"#;

#[cfg(test)]
#[path = "tests/launch_screen.rs"]
mod tests;
