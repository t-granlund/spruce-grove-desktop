"""Full-stack E2E via tauri-driver (WebDriver): launch the REAL app, verify
the ACP session comes up against the REAL CLI, send a prompt, and watch the
transcript stream. This is the last-mile test the mocked UI test cannot be.

Requires: tauri-driver on PATH, a release build at
src-tauri/target/release/spruce-grove-desktop, and the spruce-grove CLI
resolvable (fork venv or PATH). The app window will briefly appear.
"""
import json
import time
import urllib.request

DRIVER = "http://127.0.0.1:9515"
APP = "/Users/tygranlund/spruce-grove-desktop/src-tauri/target/release/spruce-grove-desktop"
CWD = "/tmp/sg-dogfood"


def req(method: str, path: str, body: dict | None = None, timeout: float = 30.0):
    data = json.dumps(body or {}).encode()
    r = urllib.request.Request(
        f"{DRIVER}{path}", data=data, method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(r, timeout=timeout) as resp:
        payload = json.loads(resp.read())
    if payload.get("status") not in (0, None):
        raise RuntimeError(f"webdriver {method} {path}: {payload}")
    value = payload.get("value")
    if isinstance(value, dict) and value.get("error"):
        raise RuntimeError(f"webdriver {method} {path}: {value}")
    return value


def wait_text(session: str, selector: str, needle: str, timeout: float = 45.0) -> str:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        el = req("POST", f"/session/{session}/element",
                 {"using": "css selector", "value": selector})
        eid = el["ELEMENT-6066-11e4-a52e-4f735466cecf"]
        text = req("GET", f"/session/{session}/element/{eid}/text")
        if needle in (text or ""):
            return text
        time.sleep(0.4)
    raise TimeoutError(f"{selector} never contained {needle!r}")


def main() -> int:
    driver = subprocess_start_driver()
    try:
        session = req("POST", "/session", {"capabilities": {"alwaysMatch": {
            "tauri:options": {"application": APP},
        }}})
        sid = session["sessionId"]
        print(f"[e2e] app launched, session {sid[:12]}…")

        cwd_el = req("POST", f"/session/{sid}/element",
                     {"using": "css selector", "value": "#cwd"})
        cwd_id = cwd_el["ELEMENT-6066-11e4-a52e-4f735466cecf"]
        req("POST", f"/session/{sid}/element/{cwd_id}/clear")
        req("POST", f"/session/{sid}/element/{cwd_id}/value",
            {"text": CWD})
        time.sleep(1.0)

        mode = wait_text(sid, "#mode", "ACP", timeout=60)
        print(f"[e2e] mode pill: {mode.strip()}")

        prompt_el = req("POST", f"/session/{sid}/element",
                        {"using": "css selector", "value": "#prompt"})
        pid = prompt_el["ELEMENT-6066-11e4-a52e-4f735466cecf"]
        req("POST", f"/session/{sid}/element/{pid}/value",
            {"text": "Reply with exactly this sentence and nothing else: E2E VOICE LINK."})
        send_el = req("POST", f"/session/{sid}/element",
                      {"using": "css selector", "value": "#send"})
        req("POST", f"/session/{sid}/element/{send_el['ELEMENT-6066-11e4-a52e-4f735466cecf']}/click")

        wait_text(sid, "#transcript", "E2E VOICE LINK", timeout=120)
        status = wait_text(sid, "#status", "done", timeout=60)
        print(f"[e2e] status: {status.strip()}")
        print("[e2e] PASS: real app + real CLI over ACP, full streaming turn")
        req("DELETE", f"/session/{sid}")
        return 0
    finally:
        driver.terminate()


def subprocess_start_driver():
    import subprocess
    proc = subprocess.Popen(
        ["tauri-driver"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    time.sleep(1.0)

    class P:
        def terminate(self):
            proc.terminate()
            proc.wait(timeout=5)

    return P()


if __name__ == "__main__":
    raise SystemExit(main())
