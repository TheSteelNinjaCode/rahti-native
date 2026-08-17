//! `rahti.native.schema.json`.
//!
//! Written beside the configuration by `init`, so an editor offers the fields
//! and refuses the misspellings without a network round trip — and so the
//! schema cannot fall behind the parser, because both are in this workspace
//! and a test compares them.

/// The schema, as JSON.
pub fn json() -> String {
    let targets = rahti_native::TARGETS
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r##"{{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "https://rahti.dev/rahti.native.schema.json",
  "title": "Rahti native packaging configuration",
  "description": "Optional native packaging for a Rahti application. Created by `cargo rahti native init`. Holds names, sizes and identifiers only: signing credentials, the session key and anything per-installation are deliberately absent and have no field to go in.",
  "type": "object",
  "additionalProperties": false,
  "required": ["schema", "productName", "identifier", "version", "targets"],
  "properties": {{
    "$schema": {{
      "type": "string",
      "description": "Path to this schema."
    }},
    "schema": {{
      "type": "integer",
      "const": {schema_version},
      "description": "The format version. A file with a version this tool does not know is refused rather than guessed at."
    }},
    "productName": {{
      "type": "string",
      "minLength": 1,
      "description": "What the installed application is called. It becomes a filename, so it cannot contain a path separator, a wildcard, or leading and trailing whitespace — `cargo rahti native init` refuses one that does."
    }},
    "identifier": {{
      "type": "string",
      "pattern": "^[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+$",
      "description": "Reverse-DNS. The Windows bundle identity and the Android package name, so every segment must be a legal Java identifier — no hyphens. Changing it makes a different application."
    }},
    "version": {{
      "type": "string",
      "pattern": "^[0-9]+\\.[0-9]+\\.[0-9]+$",
      "description": "major.minor.patch, all numeric. A Windows installer version is three numbers and Google Play orders releases by an integer derived from them, so a pre-release suffix has nowhere to go."
    }},
    "targets": {{
      "type": "array",
      "minItems": 1,
      "uniqueItems": true,
      "items": {{ "enum": [{targets}] }},
      "description": "Which packages this project builds."
    }},
    "window": {{
      "type": "object",
      "additionalProperties": false,
      "properties": {{
        "title": {{ "type": "string" }},
        "width": {{ "type": "integer", "minimum": 1 }},
        "height": {{ "type": "integer", "minimum": 1 }},
        "resizable": {{ "type": "boolean", "description": "Ignored on Android, which has no window to resize." }}
      }}
    }},
    "android": {{
      "type": "object",
      "additionalProperties": false,
      "properties": {{
        "minSdk": {{
          "type": "integer",
          "minimum": {min_sdk},
          "description": "The lowest Android API level the package installs on. Tauri 2 needs at least {min_sdk}."
        }}
      }}
    }},
    "local": {{
      "type": "string",
      "description": "Depend on a Rahti checkout by path rather than by version. For working on the framework itself; the same idea as `cargo rahti new --local`. Relative to the project root, or absolute."
    }},
    "bundle": {{
      "type": "object",
      "additionalProperties": false,
      "properties": {{
        "icons": {{ "type": "string", "description": "Where the launcher icons live, relative to the project root." }}
      }}
    }},
    "database": {{
      "type": "object",
      "additionalProperties": false,
      "properties": {{
        "mode": {{
          "enum": ["sqlite-local", "remote"],
          "description": "`sqlite-local` creates a SQLite file in application storage on first launch. `remote` leaves DATABASE_URL exactly as it is, which is what a project on PostgreSQL or MySQL needs — its database is somewhere else, and rewriting the connection string would start the application against an empty one that looked like a working one."
        }}
      }}
    }},
    "auth": {{
      "type": "object",
      "additionalProperties": false,
      "properties": {{
        "cookieName": {{
          "type": "string",
          "pattern": "^[A-Za-z0-9!#$%&'*+\\-.^_`|~]+$",
          "description": "The project's AUTH_COOKIE_NAME, so a package keeps the same session cookie as its web deployment. A name, never a key: it is in every response header the application sends. AUTH_SECRET is generated per installation on the device and has no field here."
        }}
      }}
    }},
    "security": {{
      "type": "object",
      "additionalProperties": false,
      "properties": {{
        "loopbackToken": {{
          "type": "boolean",
          "description": "Refuse requests to the embedded server that did not come from this launch of this application. 127.0.0.1 is reachable by every process on the machine; this is what keeps the port to the WebView the shell opened. On by default."
        }},
        "csp": {{
          "type": "string",
          "minLength": 1,
          "description": "The Content-Security-Policy the embedded server sends. An XSS in a native shell reaches the native command bridge as well as the session, so this is worth being strict about."
        }}
      }}
    }},
    "scaffold": {{
      "type": "object",
      "additionalProperties": {{ "type": "string" }},
      "description": "Content hashes of the generated files under native/, written by `init`. They are how a later run knows which files you have edited, and are not meant to be edited themselves."
    }}
  }}
}}
"##,
        schema_version = rahti_native::SCHEMA_VERSION,
        min_sdk = rahti_native::MIN_ANDROID_SDK,
        targets = targets,
    )
}

#[cfg(test)]
#[path = "tests/schema.rs"]
mod tests;
