//! Actually calling the portals, so "usable" is proven rather than inferred.
//!
//! Every portal call returns immediately with a Request object path and answers
//! later over that path's `Response` signal. The subscription has to exist
//! before the call, or a portal that answers instantly races us — hence the
//! handle token, which lets the path be computed up front instead of learned
//! from the reply.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::portal::{DESTINATION, OBJECT};

/// `/org/freedesktop/portal/desktop/request/<sender>/<token>`, per the
/// `org.freedesktop.portal.Request` documentation.
fn request_path(connection: &Connection, token: &str) -> zbus::Result<String> {
    let unique = connection
        .unique_name()
        .ok_or_else(|| zbus::Error::Failure(String::from("no unique bus name")))?
        .to_string();
    let sender = unique.trim_start_matches(':').replace('.', "_");
    Ok(format!("/org/freedesktop/portal/desktop/request/{sender}/{token}"))
}

fn token(seq: u32) -> String {
    format!("y5probe{}_{seq}", std::process::id())
}

/// Subscribe to a Request's `Response`, run `call`, then block for the answer.
///
/// Returns the portal's response code (0 success, 1 cancelled, 2 ended) and its
/// results map.
fn request<F>(
    connection: &Connection,
    seq: u32,
    call: F,
) -> zbus::Result<(u32, HashMap<String, OwnedValue>)>
where
    F: FnOnce(&str) -> zbus::Result<OwnedObjectPath>,
{
    let handle = token(seq);
    let path = request_path(connection, &handle)?;
    let listener = Proxy::new(connection, DESTINATION, path.as_str(), "org.freedesktop.portal.Request")?;
    let mut responses = listener.receive_signal("Response")?;

    // Portals older than the handle-token convention return a path of their own
    // choosing. Nothing to be done about the race there, but it is worth saying
    // so rather than hanging silently on the wrong path.
    let returned = call(&handle)?;
    if returned.as_str() != path {
        println!("      note: portal chose {returned} rather than the token path");
    }

    println!("      waiting for you to respond (Ctrl-C to skip)…");
    let message = responses
        .next()
        .ok_or_else(|| zbus::Error::Failure(String::from("signal stream ended")))?;
    let (code, results) = message.body().deserialize::<(u32, HashMap<String, OwnedValue>)>()?;
    Ok((code, results))
}

fn options(handle: &str) -> HashMap<&str, Value<'_>> {
    HashMap::from([("handle_token", Value::from(handle))])
}

pub fn open_file(connection: &Connection) -> zbus::Result<String> {
    let proxy = Proxy::new(connection, DESTINATION, OBJECT, "org.freedesktop.portal.FileChooser")?;
    let (code, results) = request(connection, 1, |handle| {
        proxy.call("OpenFile", &("", "portal-stress: pick any file", options(handle)))
    })?;
    Ok(describe(code, results.get("uris")))
}

pub fn screenshot(connection: &Connection) -> zbus::Result<String> {
    let proxy = Proxy::new(connection, DESTINATION, OBJECT, "org.freedesktop.portal.Screenshot")?;
    let (code, results) = request(connection, 2, |handle| {
        proxy.call("Screenshot", &("", options(handle)))
    })?;
    Ok(describe(code, results.get("uri")))
}

/// CreateSession → SelectSources → Start. Three round trips, each its own
/// Request; only the last one shows the user a picker.
pub fn screencast(connection: &Connection) -> zbus::Result<String> {
    let proxy = Proxy::new(connection, DESTINATION, OBJECT, "org.freedesktop.portal.ScreenCast")?;

    let session_token = format!("{}_session", token(3));
    let (code, results) = request(connection, 3, |handle| {
        let mut opts = options(handle);
        opts.insert("session_handle_token", Value::from(session_token.as_str()));
        proxy.call("CreateSession", &(opts,))
    })?;
    if code != 0 {
        return Ok(format!("CreateSession refused (code {code})"));
    }
    let session = results
        .get("session_handle")
        .and_then(|v| <&str>::try_from(v).ok())
        .ok_or_else(|| zbus::Error::Failure(String::from("no session_handle in reply")))?
        .to_string();
    let session = ObjectPath::try_from(session.clone())
        .map_err(|e| zbus::Error::Failure(format!("session_handle {session:?}: {e}")))?;

    // 1 = monitor, 2 = window; ask for both and let the backend offer what it has.
    let (code, _) = request(connection, 4, |handle| {
        let mut opts = options(handle);
        opts.insert("types", Value::from(3u32));
        opts.insert("multiple", Value::from(false));
        proxy.call("SelectSources", &(&session, opts))
    })?;
    if code != 0 {
        return Ok(format!("SelectSources refused (code {code})"));
    }

    let (code, results) = request(connection, 5, |handle| {
        proxy.call("Start", &(&session, "", options(handle)))
    })?;
    Ok(describe(code, results.get("streams")))
}

fn describe(code: u32, value: Option<&OwnedValue>) -> String {
    match code {
        0 => match value {
            Some(v) => format!("returned {v:?}"),
            None => String::from("succeeded"),
        },
        1 => String::from("cancelled by user"),
        other => format!("ended (code {other})"),
    }
}
