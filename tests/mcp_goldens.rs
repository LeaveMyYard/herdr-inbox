//! `herdr-inbox mcp` against the Python server's recordings
//! (`tests/fixtures/mcp/*.jsonl`, `tools/capture_mcp.py`): each session's client
//! lines are sent to the binary (environment cleared, every HERDR_* unset) and
//! its replies compared with the Python server's.
//!
//! `tools/list` is held byte for byte: the tools' order, names, descriptions and
//! both schemas of each (serialized in their key order) are the Python server's
//! exactly. Where rmcp's envelope differs (stated in `src/mcp/mod.rs`), the parts
//! a client reads are compared: an unknown method's code, a refused call's
//! `isError`, the advertised capabilities, instructions and server info.

mod mcp_rig;

use mcp_rig::{by_id, golden, of, replay};
use serde_json::{Value, json};

/// The capabilities, instructions and server name an `initialize` or discovery reply gives.
fn advertised(m: &Value) -> Value {
    let r = &m["result"];
    let info = r
        .get("serverInfo")
        .cloned()
        .unwrap_or_else(|| r["_meta"]["io.modelcontextprotocol/serverInfo"].clone());
    json!([
        r["capabilities"],
        r["instructions"],
        info,
        r.get("protocolVersion"),
        r.get("supportedVersions")
    ])
}

/// Each tool's name, description and schemas, serialized as written.
fn tool_bytes(tools: &Value) -> Vec<[String; 4]> {
    tools
        .as_array()
        .expect("a tool list")
        .iter()
        .map(|t| {
            [
                t["name"].to_string(),
                t["description"].to_string(),
                serde_json::to_string(&t["inputSchema"]).unwrap(),
                serde_json::to_string(&t["outputSchema"]).unwrap(),
            ]
        })
        .collect()
}

#[test]
fn tools_list_is_the_python_servers_byte_for_byte() {
    let recs = golden("legacy");
    let (got, _dir) = replay(&of(&recs, "client->server"));
    let want = of(&recs, "server->client");
    let (got, want) = (by_id(&got, &json!(1)), by_id(&want, &json!(1)));
    let names: Vec<&str> = want["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "ask_operator",
            "alert_operator",
            "withdraw_alert",
            "check_answers",
            "list_my_questions",
            "withdraw_question"
        ]
    );
    let (g, w) = (
        tool_bytes(&got["result"]["tools"]),
        tool_bytes(&want["result"]["tools"]),
    );
    for (g, w) in g.iter().zip(&w) {
        assert_eq!(g, w);
    }
    assert_eq!(g.len(), w.len());
    assert_eq!(got, want); // and nothing else in the reply
}

#[test]
fn the_legacy_handshake_answers_and_refuses_as_python_does() {
    let recs = golden("legacy");
    let (got, _dir) = replay(&of(&recs, "client->server"));
    let want = of(&recs, "server->client");
    assert_eq!(
        advertised(by_id(&got, &json!(0))),
        advertised(by_id(&want, &json!(0)))
    );
    assert_eq!(
        by_id(&got, &json!(2))["error"]["code"],
        by_id(&want, &json!(2))["error"]["code"]
    );
    assert_eq!(by_id(&got, &json!(3)), by_id(&want, &json!(3))); // an unknown tool
    assert_eq!(by_id(&got, &json!(4)), by_id(&want, &json!(4))); // ping
    // refused by the schema: pydantic's text without its codes, inputs and links
    for id in 5..=15 {
        assert_refused_alike(by_id(&got, &json!(id)), by_id(&want, &json!(id)), id);
    }
}

/// pydantic's refusal as the Rust server words it: each `field` line, and each
/// reason line cut before ` [type=`; the `For further information` lines dropped.
fn without_codes(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with("For further information"))
        .map(|l| l.split(" [type=").next().unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_refused_alike(got: &Value, want: &Value, id: i64) {
    assert_eq!(got["result"]["isError"], json!(true), "call {id}");
    assert_eq!(want["result"]["isError"], json!(true), "call {id}");
    let text = |m: &Value| {
        m["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(text(got), without_codes(&text(want)), "call {id}");
}

#[test]
fn claude_codes_2026_requests_get_the_python_servers_answers() {
    let recs = golden("claude-code");
    let (got, _dir) = replay(&of(&recs, "client->server"));
    let want = of(&recs, "server->client");
    for id in [json!("server-discover-probe-1"), json!(0)] {
        assert_eq!(
            advertised(by_id(&got, &id)),
            advertised(by_id(&want, &id)),
            "{id}"
        );
    }
    let ack = |msgs: &[Value]| {
        msgs.iter()
            .find(|m| m["method"] == "notifications/subscriptions/acknowledged")
            .cloned()
    };
    assert_eq!(ack(&got), ack(&want));
    assert_eq!(by_id(&got, &json!(1))["result"]["prompts"], json!([]));
    assert_eq!(by_id(&got, &json!(2))["result"]["resources"], json!([]));
    // the same tools: Python's own key order differs under 2026-07-28 (`type` first, the
    // result schema's keys sorted), so here the schemas are compared as JSON values; the
    // names and descriptions are compared byte for byte
    let (g, w) = (
        &by_id(&got, &json!(3))["result"]["tools"],
        &by_id(&want, &json!(3))["result"]["tools"],
    );
    assert_eq!(g, w);
    let text = |ts: &Value| -> Vec<[String; 2]> {
        tool_bytes(ts)
            .into_iter()
            .map(|[n, d, _, _]| [n, d])
            .collect()
    };
    assert_eq!(text(g), text(w));
}

#[test]
fn ask_operators_refusals_are_the_python_receipts() {
    let recs = golden("outside-herdr");
    let (got, _dir) = replay(&of(&recs, "client->server"));
    let want = of(&recs, "server->client");
    // 12-14: JSON text in a non-`str` argument is read as JSON first (FastMCP's pre-pass)
    for id in (2..=5).chain(12..=14) {
        assert_eq!(
            by_id(&got, &json!(id)),
            by_id(&want, &json!(id)),
            "call {id}"
        );
    }
}

/// Tool calls as Claude Code makes them (2026-07-28, with its `_meta`): the same
/// result, `resultType` included. Python adds `_meta.io.modelcontextprotocol/serverInfo`
/// to every result; rmcp does not (a stated deviation: discovery carries it).
#[test]
fn claude_codes_tool_calls_get_the_python_servers_results() {
    let recs = golden("claude-code-calls");
    let (got, _dir) = replay(&of(&recs, "client->server"));
    let want = of(&recs, "server->client");
    let bare = |m: &Value| {
        let mut r = m["result"].clone();
        r.as_object_mut().unwrap().remove("_meta");
        r
    };
    assert_eq!(
        bare(by_id(&got, &json!(100))),
        bare(by_id(&want, &json!(100)))
    );
    assert_eq!(
        bare(by_id(&got, &json!(102))),
        bare(by_id(&want, &json!(102)))
    );
    let (g, w) = (by_id(&got, &json!(101)), by_id(&want, &json!(101)));
    assert_refused_alike(g, w, 101);
    assert_eq!(g["result"]["resultType"], w["result"]["resultType"]);
}
