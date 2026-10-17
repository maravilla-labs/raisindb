//! Plugin bindings — the part of the `raisin.*` surface a loaded
//! `FunctionBindingPlugin` contributes (today: `raisin.media.*`).
//!
//! Hand-written, not in `generated.rs`: these methods are not in the static
//! registry the generator reads; they exist only when a plugin is loaded. The
//! host routes them exactly as the JavaScript gateway does
//! (`raisin-functions/src/runtime/wasm/bindings.rs` `route_plugin_call`): the
//! argument ARRAY goes to the plugin unchanged, an absent optional namespace
//! answers `{ok:false, reason:"plugin_absent", method}` instead of failing, and
//! a plugin's own soft failures (`{ok:false, reason}`) are values, not errors.
//! A `{"error": true}` envelope is an `Err`, as for every host call.

use serde::Serialize;
use serde_json::Value;

/// Call any plugin binding by its fully-qualified name (`"media.doc.toPdf"`),
/// with the positional arguments the JavaScript form takes.
pub fn call(method: &str, args: Vec<Value>) -> crate::Result<Value> {
    let raw = crate::host::call(method, &Value::Array(args).to_string())?;
    crate::wire::decode_json(&raw)
}

/// What the server can do: the loaded plugins and every method they service.
/// The same data JavaScript reads as `raisin.capabilities`.
pub fn capabilities() -> crate::Result<Value> {
    let raw = crate::host::call("capabilities.get", "[]")?;
    crate::wire::decode_json(&raw)
}

/// `raisin.capabilities.has(method)`.
pub fn has(method: &str) -> crate::Result<bool> {
    Ok(capabilities()?
        .get("methods")
        .and_then(Value::as_array)
        .map_or(false, |m| m.iter().any(|x| x.as_str() == Some(method))))
}

fn to_value(v: impl Serialize) -> crate::Result<Value> {
    Ok(serde_json::to_value(v)?)
}

/// `(srcKey, opts)`: a stored object plus kind-specific options (`null` when absent).
fn keyed(method: &str, src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
    call(
        method,
        vec![
            Value::String(src_key.to_string()),
            opts.unwrap_or(Value::Null),
        ],
    )
}

/// `raisin.media.*` (maravilla media plugin). Every submit returns
/// `{jobId, outputKey, status}` without blocking; poll [`media::job`] (from a
/// later invocation or a `maravilla:MediaJob` trigger) and read the result with
/// [`media::fetch`]. Request keys may be written camelCase, as in JavaScript.
pub mod media {
    use super::*;

    /// `raisin.media.job(jobId)` → `{jobId, status, percent, outputKey, sizeBytes?, error?}`.
    pub fn job(job_id: &str) -> crate::Result<Value> {
        call("media.job", vec![Value::String(job_id.to_string())])
    }

    /// `raisin.media.fetch(outputKey, opts)` → `{base64, mimeType, outputKey, bytes}`
    /// (`{ok:false, reason:"too_large"}` past the cap).
    pub fn fetch(output_key: &str, opts: Option<Value>) -> crate::Result<Value> {
        keyed("media.fetch", output_key, opts)
    }

    /// `raisin.media.upload(input)` — `{base64, mimeType, name}` or a bare base64 string → `{key}`.
    pub fn upload(input: impl Serialize) -> crate::Result<Value> {
        call("media.upload", vec![to_value(input)?])
    }

    /// `raisin.media.uploadFromUrl(url, opts)` → `{key, bytes}`.
    pub fn upload_from_url(url: &str, opts: Option<Value>) -> crate::Result<Value> {
        keyed("media.uploadFromUrl", url, opts)
    }

    /// DEPRECATED blocking `raisin.media.screenshot(request)`; use [`browser::screenshot`].
    pub fn screenshot(request: impl Serialize) -> crate::Result<Value> {
        call("media.screenshot", vec![to_value(request)?])
    }

    /// `raisin.media.browser.*` — headless browser renders of a URL or a path on
    /// the tenant's own deployment. Each takes one request object.
    pub mod browser {
        use super::super::*;

        /// `raisin.media.browser.screenshot(request)`.
        pub fn screenshot(request: impl Serialize) -> crate::Result<Value> {
            call("media.browser.screenshot", vec![to_value(request)?])
        }
        /// `raisin.media.browser.pdf(request)`.
        pub fn pdf(request: impl Serialize) -> crate::Result<Value> {
            call("media.browser.pdf", vec![to_value(request)?])
        }
        /// `raisin.media.browser.extractHtml(request)`: the rendered DOM
        /// (`documentElement.outerHTML`), after `waitFor` (selector / networkIdle / ms).
        pub fn extract_html(request: impl Serialize) -> crate::Result<Value> {
            call("media.browser.extractHtml", vec![to_value(request)?])
        }
    }

    /// `raisin.media.video.render(request)` (frames renderer).
    pub mod video {
        use super::super::*;

        /// `raisin.media.video.render(request)`.
        pub fn render(request: impl Serialize) -> crate::Result<Value> {
            call("media.video.render", vec![to_value(request)?])
        }
        /// `raisin.media.video.transcode(srcKey, opts)`.
        pub fn transcode(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.video.transcode", src_key, opts)
        }
        /// `raisin.media.video.thumbnail(srcKey, opts)`.
        pub fn thumbnail(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.video.thumbnail", src_key, opts)
        }
        /// `raisin.media.video.extractFrames(srcKey, opts)`.
        pub fn extract_frames(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.video.extractFrames", src_key, opts)
        }
        /// `raisin.media.video.extractAudio(srcKey, opts)`.
        pub fn extract_audio(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.video.extractAudio", src_key, opts)
        }
        /// `raisin.media.video.watermark(srcKey, opts)`.
        pub fn watermark(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.video.watermark", src_key, opts)
        }
    }

    /// `raisin.media.image.*`.
    pub mod image {
        use super::super::*;

        /// `raisin.media.image.resize(srcKey, opts)`.
        pub fn resize(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.image.resize", src_key, opts)
        }
        /// `raisin.media.image.watermark(srcKey, opts)`.
        pub fn watermark(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.image.watermark", src_key, opts)
        }
        /// `raisin.media.image.detectFaces(srcKey, opts)`.
        pub fn detect_faces(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.image.detectFaces", src_key, opts)
        }
        /// `raisin.media.image.ocr(srcKey, opts)`.
        pub fn ocr(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.image.ocr", src_key, opts)
        }
    }

    /// `raisin.media.pdf.*`.
    pub mod pdf {
        use super::super::*;

        /// `raisin.media.pdf.stamp(srcKey, opts)`.
        pub fn stamp(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.pdf.stamp", src_key, opts)
        }
        /// `raisin.media.pdf.merge([first, ...rest], opts)`: the wire form is
        /// `(first, {append: rest, ...opts})`, as the JavaScript wrapper sends it.
        pub fn merge(src_keys: &[&str], opts: Option<Value>) -> crate::Result<Value> {
            let mut o = match opts {
                Some(Value::Object(m)) => m,
                _ => serde_json::Map::new(),
            };
            let mut append: Vec<Value> = src_keys
                .iter()
                .skip(1)
                .map(|k| Value::String((*k).to_string()))
                .collect();
            if let Some(Value::Array(more)) = o.remove("append") {
                append.extend(more);
            }
            o.insert("append".into(), Value::Array(append));
            let first = src_keys
                .first()
                .map(|k| Value::String((*k).to_string()))
                .unwrap_or(Value::Null);
            call("media.pdf.merge", vec![first, Value::Object(o)])
        }
    }

    /// `raisin.media.doc.*` (LibreOffice conversions).
    pub mod doc {
        use super::super::*;

        /// `raisin.media.doc.toPdf(srcKey, opts)`.
        pub fn to_pdf(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.doc.toPdf", src_key, opts)
        }
        /// `raisin.media.doc.thumbnail(srcKey, opts)`.
        pub fn thumbnail(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.doc.thumbnail", src_key, opts)
        }
        /// `raisin.media.doc.convert(srcKey, opts)`.
        pub fn convert(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.doc.convert", src_key, opts)
        }
        /// `raisin.media.doc.toMarkdown(srcKey, opts)`.
        pub fn to_markdown(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.doc.toMarkdown", src_key, opts)
        }
        /// `raisin.media.doc.toHtml(srcKey, opts)`.
        pub fn to_html(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.doc.toHtml", src_key, opts)
        }
        /// `raisin.media.doc.replaceImages(srcKey, opts)`.
        pub fn replace_images(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.doc.replaceImages", src_key, opts)
        }
        /// `raisin.media.doc.insertQrCode(srcKey, opts)`.
        pub fn insert_qr_code(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.doc.insertQrCode", src_key, opts)
        }
        /// `raisin.media.doc.templateMerge(srcKey, opts)`.
        pub fn template_merge(src_key: &str, opts: Option<Value>) -> crate::Result<Value> {
            keyed("media.doc.templateMerge", src_key, opts)
        }
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use crate::testing::{with_mock, MockHost};
    use serde_json::json;

    #[test]
    fn media_calls_carry_the_javascript_argument_arrays() {
        let mock = MockHost::new()
            .expect_any(
                "media.browser.extractHtml",
                Ok(json!({ "jobId": "j1", "outputKey": "o.html", "status": "queued" }).to_string()),
            )
            .expect_any(
                "media.image.resize",
                Ok(json!({ "jobId": "j2" }).to_string()),
            )
            .expect_any("media.pdf.merge", Ok(json!({ "jobId": "j3" }).to_string()))
            .expect_any(
                "media.fetch",
                Ok(json!({ "ok": false, "reason": "too_large" }).to_string()),
            );
        let (r, mock) = with_mock(mock, || {
            let a = super::media::browser::extract_html(
                json!({ "path": "/x", "waitFor": { "selector": "#a" } }),
            )?;
            super::media::image::resize("k", None)?;
            super::media::pdf::merge(&["a", "b", "c"], Some(json!({ "append": ["d"] })))?;
            let f = super::media::fetch("o.html", None)?;
            Ok::<_, crate::Error>((a, f))
        });
        let (a, f) = r.unwrap();
        assert_eq!(a["jobId"], "j1");
        assert_eq!(
            f["reason"], "too_large",
            "a plugin's soft failure is a value, not an Err"
        );
        let calls = mock.calls();
        assert_eq!(
            calls[0].args,
            json!([{ "path": "/x", "waitFor": { "selector": "#a" } }]).to_string()
        );
        assert_eq!(calls[1].args, json!(["k", null]).to_string());
        assert_eq!(
            calls[2].args,
            json!(["a", { "append": ["b", "c", "d"] }]).to_string()
        );
    }

    #[test]
    fn capabilities_answer_has() {
        let mock = MockHost::new().expect_any(
            "capabilities.get",
            Ok(json!({ "plugins": ["media"], "methods": ["media.job"] }).to_string()),
        );
        let (r, _) = with_mock(mock, || Ok::<_, crate::Error>((super::has("media.job")?,)));
        assert!(r.unwrap().0);
    }
}
