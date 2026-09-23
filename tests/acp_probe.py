"""ACP contract probe: drive `spruce-grove --acp` over stdio and record the dialect.

This is the ground-truth explorer for the desktop's Rust ACP client, and the
generator for the contract fixtures (bead SPRUCE-GROVE-OS-tx3). It speaks
newline-delimited JSON-RPC exactly like the real client will.

Usage:
    python tests/acp_probe.py [--with-tools]

Prints every message observed, grouped by direction, then a summary of the
dialect (methods, update variants, prompt/permission shapes).
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

# The CLI writes terminal color escapes on stdout at boot, then the first
# JSON-RPC message on the SAME physical line (observed: ~256 bytes of OSC).
# A raw json.loads chokes on that prefix, so every line is de-ANSI'd first —
# the same rule the Rust client now applies in acp.rs::parse_line.
_OSC = re.compile(r"\x1b\][^\x07]*\x07")
_CSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


def parse_wire_line(line: str):
    """Strip ANSI from one stdout line and parse it as JSON-RPC, or None."""
    cleaned = _CSI.sub("", _OSC.sub("", line)).strip()
    if not cleaned:
        return None
    try:
        return json.loads(cleaned)
    except json.JSONDecodeError:
        start = cleaned.find("{")
        if start < 0:
            return None
        try:
            return json.loads(cleaned[start:])
        except json.JSONDecodeError:
            return None

CWD = os.environ.get("ACP_PROBE_CWD", "/tmp/sg-dogfood")
TIMEOUT = float(os.environ.get("ACP_PROBE_TIMEOUT", "90"))
LOG = Path("/tmp/sg-dogfood/acp_probe_transcript.jsonl")


class AcpProbe:
    def __init__(self, with_tools: bool) -> None:
        cmd = ["spruce-grove", "--acp"]
        if not with_tools:
            cmd.append("--no-tools")
        self.proc = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            cwd=CWD,
            text=True,
            bufsize=1,
        )
        self.seen: list[dict] = []
        self.pending: dict[int, threading.Event] = {}
        self.responses: dict[int, dict] = {}
        self.requests_from_agent: list[dict] = []
        self._lock = threading.Lock()
        self._reader = threading.Thread(target=self._read_loop, daemon=True)
        self._reader.start()

    def _read_loop(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            msg = parse_wire_line(line)
            if msg is None:
                continue
            with self._lock:
                self.seen.append(msg)
                LOG.parent.mkdir(parents=True, exist_ok=True)
                with LOG.open("a") as fh:
                    fh.write(json.dumps(msg) + "\n")
            if "id" in msg and ("result" in msg or "error" in msg):
                ev = self.pending.pop(msg["id"], None)
                if ev:
                    self.responses[msg["id"]] = msg
                    ev.set()
            elif msg.get("method") and "id" in msg:
                # request FROM the agent (e.g. session/request_permission, fs/*)
                with self._lock:
                    self.requests_from_agent.append(msg)

    def send(self, msg: dict) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def request(self, method: str, params: dict, timeout: float = 30.0) -> dict:
        with self._lock:
            next_id = max([m.get("id", 0) for m in self.seen if isinstance(m.get("id"), int)], default=0) + 1
        rid = next_id
        ev = threading.Event()
        self.pending[rid] = ev
        self.send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        if not ev.wait(timeout):
            raise TimeoutError(f"{method} timed out after {timeout}s")
        return self.responses[rid]

    def wait_for(self, predicate, timeout: float) -> list[dict]:
        deadline = time.monotonic() + timeout
        matched: list[dict] = []
        while time.monotonic() < deadline:
            with self._lock:
                matched = [m for m in self.seen if predicate(m)]
            if matched:
                return matched
            time.sleep(0.05)
        return matched

    def close(self) -> None:
        try:
            self.proc.terminate()
            self.proc.wait(timeout=5)
        except Exception:
            self.proc.kill()


def probe_lifecycle() -> int:
    """Durability proof at protocol level: remember across process death.

    Session A: new -> prompt (plant a marker word) -> close.
    Process B: fresh agent -> session/load(same sessionId) -> prompt asking
    for the marker. If B recalls it, quick-resume durability is proven for
    the desktop's relaunch path.
    """
    marker = f"GROVE-DURABILITY-{time.strftime('%H%M%S')}"
    probe = AcpProbe(with_tools=False)
    try:
        probe.request("initialize", {"protocolVersion": 1, "clientCapabilities": {}}, 30)
        new = probe.request("session/new", {"cwd": CWD, "mcpServers": []}, 30)
        sid = new["result"]["sessionId"]

        def ask(p: AcpProbe, sid_arg: str, text: str) -> str:
            rid = int(time.time() * 1000) % 100000
            p.send({"jsonrpc": "2.0", "id": rid,
                    "method": "session/prompt",
                    "params": {"sessionId": sid_arg,
                               "prompt": [{"type": "text", "text": text}]}})
            p.wait_for(
                lambda m: m.get("id") == rid and "result" in m, TIMEOUT
            )
            updates = [m for m in p.seen if m.get("method") == "session/update"]
            return "".join(
                (u["params"]["update"].get("content", {}) or {}).get("text", "")
                for u in updates
                if u["params"]["update"].get("sessionUpdate") == "agent_message_chunk"
            )

        ask(probe, sid, f"Remember this exact word for later: {marker}. Reply OK.")
        print(f"[lifecycle] marker planted: {marker}")
        probe.close()

        probe2 = AcpProbe(with_tools=False)
        try:
            probe2.request("initialize", {"protocolVersion": 1, "clientCapabilities": {}}, 30)
            load = probe2.request(
                "session/load", {"cwd": CWD, "sessionId": sid, "mcpServers": []}, 30
            )
            print(f"[lifecycle] session/load ok: {json.dumps(load.get('result', {}))[:120]}")
            answer = ask(
                probe2, sid,
                "What exact word did I ask you to remember earlier in this session? "
                "Reply with only that word.",
            )
            print(f"[lifecycle] recalled answer: {answer.strip()[:200]}")
            if marker in answer:
                print("[lifecycle] PASS: session survived process death")
                return 0
            print("[lifecycle] FAIL: marker not recalled")
            return 1
        finally:
            probe2.close()
    finally:
        probe.close()


def main() -> int:
    if "--lifecycle" in sys.argv:
        return probe_lifecycle()
    with_tools = "--with-tools" in sys.argv
    Path(LOG).unlink(missing_ok=True)
    probe = AcpProbe(with_tools)
    dialect: dict = {"with_tools": with_tools}

    try:
        init = probe.request(
            "initialize",
            {"protocolVersion": 1, "clientCapabilities": {}},
            timeout=30,
        )
        print("== initialize result ==")
        print(json.dumps(init.get("result", init), indent=1)[:600])
        dialect["initialize_result"] = init.get("result", init)

        new = probe.request("session/new", {"cwd": CWD, "mcpServers": []}, timeout=30)
        print("== session/new result ==")
        print(json.dumps(new.get("result", new), indent=1)[:300])
        dialect["session_new_result"] = new.get("result", new)
        session_id = new["result"]["sessionId"]

        prompt = "Reply with exactly this sentence and nothing else: ACP ONLINE."
        probe.send(
            {
                "jsonrpc": "2.0",
                "id": 9001,
                "method": "session/prompt",
                "params": {
                    "sessionId": session_id,
                    "prompt": [{"type": "text", "text": prompt}],
                },
            }
        )
        # wait for the turn to COMPLETE before harvesting updates, so we see
        # everything the agent emitted during the turn (chunks, tool calls...)
        resp = probe.wait_for(
            lambda m: m.get("id") == 9001 and ("result" in m or "error" in m), TIMEOUT
        )
        if resp:
            print("== session/prompt response ==")
            print(json.dumps(resp[0].get("result", resp[0]), indent=1)[:200])
            dialect["prompt_response"] = resp[0].get("result", resp[0])

        updates = [m for m in probe.seen if m.get("method") == "session/update"]
        variants: dict[str, int] = {}
        samples: dict[str, dict] = {}
        text_out = []
        for u in updates:
            upd = u.get("params", {}).get("update", {})
            kind = upd.get("sessionUpdate", "?")
            variants[kind] = variants.get(kind, 0) + 1
            samples.setdefault(kind, upd)
            if kind == "agent_message_chunk":
                content = upd.get("content", {})
                text_out.append(content.get("text", ""))
        print("== session/update variants seen ==")
        print(json.dumps(variants, indent=1))
        dialect["update_variants"] = variants
        dialect["update_samples"] = samples
        print("== assembled agent text ==")
        print("".join(text_out)[:400])

        if probe.requests_from_agent:
            print("== requests FROM agent ==")
            for r in probe.requests_from_agent[:3]:
                print(json.dumps(r)[:300])
            dialect["agent_requests"] = probe.requests_from_agent[:3]

        Path("/tmp/sg-dogfood/acp_dialect.json").write_text(
            json.dumps(dialect, indent=1, default=str)
        )
        print(f"\ndialect saved: /tmp/sg-dogfood/acp_dialect.json")
        print(f"raw transcript: {LOG} ({len(probe.seen)} messages)")
        return 0
    finally:
        probe.close()


if __name__ == "__main__":
    raise SystemExit(main())
