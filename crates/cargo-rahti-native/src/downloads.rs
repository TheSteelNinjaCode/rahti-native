//! File downloads on Android.
//!
//! ## The bug this exists for
//!
//! An Android WebView does nothing with a download. A link with `download`
//! on it, or a response sent as an attachment, reaches a `DownloadListener`
//! if the application set one and is dropped if it did not — and Tauri's
//! generated WebView sets none. So "Export", "Download invoice" and every
//! `<a download>` in a Rahti application work in a browser, work in the
//! Windows package, and on a phone are a button that does nothing at all.
//!
//! ## What is written
//!
//! `RahtiDownloads.kt`, beside `MainActivity.kt`, and one line in its
//! `onCreate`. The listener fetches the file itself and saves it to the
//! device's Downloads.
//!
//! ## Why it fetches rather than handing the URL to the system
//!
//! Android's `DownloadManager` runs in another process. The embedded server
//! refuses every request without this launch's cookie, so the system would
//! have to be given that cookie and the session cookie beside it. Fetching
//! from the application's own process keeps both where they already are.
//!
//! Only the page's own origin is fetched: a link to anywhere else has already
//! been sent to the user's browser, which downloads perfectly well.
//!
//! Nothing here is a native command. A page cannot name where a file goes;
//! it lands in Downloads under the name the response gave it, as it would
//! from a browser.

use std::path::Path;

use rahti_native::NativeError;

use crate::launch_screen::{kotlin_package, with_on_create, write_if_changed};

/// Marks the line this tool adds to `MainActivity.kt`.
const MARKER: &str = "rahti-native: save downloads";

/// What goes into `onCreate`, after `super.onCreate`.
const CODE: &str = "    // rahti-native: save downloads to the device's Downloads.
    RahtiDownloads.install(this)
";

/// Beside `MainActivity.kt`, in its package.
pub(crate) const CLASS_FILE: &str = "RahtiDownloads.kt";

/// Make a download in the page save a file.
pub(crate) fn install(generated: &Path) -> Result<(), NativeError> {
    let refuse = |path: &Path, what: &str| {
        NativeError::at(
            "android",
            path,
            format!(
                "{what}, so downloads were not wired up and a download link in a page will do \
                 nothing.\n  \
                 Delete native/gen/android to generate it again."
            ),
        )
    };

    let Some(path) = crate::run::find_main_activity(&generated.join("app/src/main/java")) else {
        return Err(refuse(
            generated,
            "this Android project has no MainActivity.kt",
        ));
    };
    let source =
        std::fs::read_to_string(&path).map_err(|e| NativeError::io("android", &path, e))?;

    let patched = with_downloads(&source).ok_or_else(|| {
        refuse(
            &path,
            "this MainActivity is not one `cargo rahti native` recognises",
        )
    })?;
    let Some(package) = kotlin_package(&source) else {
        return Err(refuse(&path, "this MainActivity names no package"));
    };

    let mut changed = write_if_changed(&path.with_file_name(CLASS_FILE), &class(package))?;
    changed |= write_if_changed(&path, &patched)?;
    if changed {
        println!("  android: downloads in a page are saved to Downloads");
    }
    Ok(())
}

/// `MainActivity.kt` with the downloads line in it, or `None` for a shape
/// this tool does not recognise. Idempotent.
pub(crate) fn with_downloads(source: &str) -> Option<String> {
    with_on_create(source, MARKER, CODE, true)
}

/// `RahtiDownloads.kt`, in the application's package.
pub(crate) fn class(package: &str) -> String {
    format!("package {package}\n{CLASS_BODY}")
}

const CLASS_BODY: &str = r#"
// Written by `cargo rahti native` on every Android build. Do not edit.

import android.app.Activity
import android.content.ContentValues
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.URLUtil
import android.webkit.WebView
import android.widget.Toast
import java.io.File
import java.io.OutputStream
import java.net.HttpURLConnection
import java.net.URL
import kotlin.concurrent.thread

/**
 * Saves what a page downloads.
 *
 * A WebView drops a download unless it has a listener, so a link with
 * `download` on it did nothing. This one fetches the file from the
 * application's own server, with the WebView's cookies, and puts it in the
 * device's Downloads.
 */
internal class RahtiDownloads private constructor(private val activity: Activity) {
  private val handler = Handler(Looper.getMainLooper())
  private var attempts = 0

  private fun findWebView(view: View?): WebView? {
    if (view is WebView) return view
    if (view is ViewGroup) {
      for (index in 0 until view.childCount) {
        findWebView(view.getChildAt(index))?.let { return it }
      }
    }
    return null
  }

  /** The WebView is created after `onCreate`, once the server is up. */
  private fun attach() {
    val webView = findWebView(activity.findViewById(android.R.id.content))
    if (webView == null) {
      attempts += 1
      if (attempts < MAX_ATTEMPTS && !activity.isDestroyed) {
        handler.postDelayed(::attach, POLL_MS)
      }
      return
    }
    webView.setDownloadListener { url, userAgent, contentDisposition, mimeType, _ ->
      val page = webView.url
      if (page != null && sameOrigin(url, page)) {
        val name = URLUtil.guessFileName(url, contentDisposition, mimeType)
        thread(name = "rahti-download") { save(url, userAgent, mimeType, name) }
      }
    }
  }

  /** Only the application's own server: its cookies go nowhere else. */
  private fun sameOrigin(url: String, page: String): Boolean {
    val a = Uri.parse(url)
    val b = Uri.parse(page)
    val web = a.scheme == "http" || a.scheme == "https"
    return web && a.scheme == b.scheme && a.host == b.host && a.port == b.port
  }

  private fun save(url: String, userAgent: String?, mimeType: String?, name: String) {
    var connection: HttpURLConnection? = null
    try {
      connection = (URL(url).openConnection() as HttpURLConnection).apply {
        CookieManager.getInstance().getCookie(url)?.let { setRequestProperty("Cookie", it) }
        userAgent?.let { setRequestProperty("User-Agent", it) }
        connectTimeout = TIMEOUT_MS
        readTimeout = TIMEOUT_MS
      }
      if (connection.responseCode !in 200..299) {
        throw java.io.IOException("HTTP ${connection.responseCode}")
      }
      val type = mimeType?.takeIf { it.isNotBlank() } ?: connection.contentType
      connection.inputStream.use { input ->
        open(name, type) { output -> input.copyTo(output) }
      }
      say("Saved $name to Downloads")
    } catch (error: Exception) {
      android.util.Log.w("RahtiDownloads", "could not save $name", error)
      say("Could not download $name")
    } finally {
      connection?.disconnect()
    }
  }

  /** Downloads, where a file manager and other applications can find it. */
  private fun open(name: String, mimeType: String?, write: (OutputStream) -> Unit) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      val resolver = activity.contentResolver
      val values = ContentValues().apply {
        put(MediaStore.Downloads.DISPLAY_NAME, name)
        mimeType?.let { put(MediaStore.Downloads.MIME_TYPE, it.substringBefore(';').trim()) }
        put(MediaStore.Downloads.IS_PENDING, 1)
      }
      val item = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
        ?: throw java.io.IOException("Downloads refused the file")
      try {
        (resolver.openOutputStream(item) ?: throw java.io.IOException("no stream")).use(write)
        values.clear()
        values.put(MediaStore.Downloads.IS_PENDING, 0)
        resolver.update(item, values, null, null)
      } catch (error: Exception) {
        resolver.delete(item, null, null)
        throw error
      }
      return
    }
    // Before Android 10 the shared folder needs a storage permission; the
    // application's own Downloads folder does not.
    val directory = activity.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS)
      ?: throw java.io.IOException("no storage")
    var file = File(directory, name)
    var copy = 1
    while (file.exists()) {
      val stem = name.substringBeforeLast('.', name)
      val extension = name.substringAfterLast('.', "")
      val suffix = if (extension.isEmpty()) "" else ".$extension"
      file = File(directory, "$stem ($copy)$suffix")
      copy += 1
    }
    file.outputStream().use(write)
  }

  private fun say(message: String) {
    handler.post { Toast.makeText(activity, message, Toast.LENGTH_LONG).show() }
  }

  companion object {
    private const val POLL_MS = 100L
    private const val MAX_ATTEMPTS = 600
    private const val TIMEOUT_MS = 30_000

    /** From `MainActivity.onCreate`. */
    fun install(activity: Activity) {
      RahtiDownloads(activity).attach()
    }
  }
}
"#;

#[cfg(test)]
#[path = "tests/downloads.rs"]
mod tests;
