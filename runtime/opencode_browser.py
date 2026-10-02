"""OpenCode browser host protocol v1, backed by the shared WebDriver connection."""

import argparse
import base64
import io
import json
import signal
import sys
import time
import uuid
from pathlib import Path

from browser_connection import WebDriver

OPERATIONS = [
    "tabs.list", "tabs.open", "tabs.focus", "tabs.close",
    "navigate", "back", "forward", "reload", "stop", "frames",
    "snapshot", "find", "evaluate", "click", "hover", "drag",
    "fill", "fill_form", "select", "check", "press", "scroll", "wait",
    "screenshot", "dialog", "files.upload", "files.drop", "files.list", "files.get",
]
ELEMENT = "element-6066-11e4-a52e-4f735466cecf"
MAX_FILE_BYTES = 5 * 1024 * 1024
MAX_LINE_BYTES = 8 * 1024 * 1024
KEYS = {
    "Enter": "\ue007", "Tab": "\ue004", "Escape": "\ue00c", "Backspace": "\ue003",
    "Delete": "\ue017", "ArrowUp": "\ue013", "ArrowDown": "\ue015", "ArrowLeft": "\ue012",
    "ArrowRight": "\ue014", "Home": "\ue011", "End": "\ue010", "PageUp": "\ue00e",
    "PageDown": "\ue00f", "Space": " ", "Shift": "\ue008", "Control": "\ue009",
    "Alt": "\ue00a", "Meta": "\ue03d",
    **{f"F{i}": chr(0xE030 + i) for i in range(1, 13)},
}


class Host:
    def __init__(self, driver):
        self.driver = driver
        self.tabs = {}
        self.focused = None
        self.next_ref = 0
        self.current_handle = None

    def inventory(self):
        handles = self.driver.call("GET", "/window/handles")
        for handle in handles:
            if not any(tab["handle"] == handle for tab in self.tabs.values()):
                self.tabs["tab_" + str(uuid.uuid4())] = {
                    "handle": handle, "generation": 0, "document": None, "refs": {}, "files": {},
                    "history": [], "position": -1,
                }
        self.tabs = {key: tab for key, tab in self.tabs.items() if tab["handle"] in handles}
        if self.focused not in self.tabs:
            self.focused = next(iter(self.tabs), None)
        return {"tabs": [self.state(key) for key in self.tabs], "focusedTabID": self.focused}

    def switch(self, tab_id, frame=None):
        if tab_id not in self.tabs:
            raise ValueError("Tab does not belong to this session; call browser.tabs.list")
        if self.current_handle != self.tabs[tab_id]["handle"]:
            self.driver.call("POST", "/window", {"handle": self.tabs[tab_id]["handle"]})
            self.current_handle = self.tabs[tab_id]["handle"]
        try:
            self.driver.call("POST", "/frame", {"id": None})
        except RuntimeError as error:
            if "unexpected alert open" not in str(error):
                raise
        if frame and frame != "main":
            for index in frame.removeprefix("main/").split("/"):
                self.driver.call("POST", "/frame", {"id": int(index)})
        return self.tabs[tab_id]

    def state(self, tab_id):
        tab = self.switch(tab_id)
        try:
            observed = self.driver.script("""
                return {url:location.href,title:document.title,loading:document.readyState !== 'complete',
                  document:performance.timeOrigin};
            """)
        except RuntimeError as error:
            if "unsupported operation" in str(error) and "privileged browsing contexts" in str(error):
                # Floorp's initial about:home/newtab can forbid page scripts.
                # Native metadata remains readable; never enable privileged
                # scripting just to inventory the selected browser's tabs.
                url = self.driver.call("GET", "/url")
                observed = {"url": url, "title": self.driver.call("GET", "/title"),
                            "loading": False, "document": ("privileged", url)}
            elif "unexpected alert open" in str(error):
                observed = {"url": tab["history"][tab["position"]] if tab["history"] else "about:blank",
                            "title": "", "loading": False}
            else:
                raise
        if "document" in observed:
            if observed["document"] != tab["document"]:
                tab["generation"] += 1
                tab["document"] = observed["document"]
                tab["refs"].clear()
            history = tab["history"]
            if not history or history[tab["position"]] != observed["url"]:
                tab["history"] = history[:tab["position"] + 1] + [observed["url"]]
                tab["position"] += 1
        return {"id": tab_id, "url": observed["url"][:16384], "title": observed["title"][:2048],
                "loading": observed["loading"], "generation": tab["generation"],
                "canGoBack": tab["position"] > 0, "canGoForward": tab["position"] < len(tab["history"]) - 1}

    def element(self, tab, ref):
        element = tab["refs"].get(ref.removeprefix("@"))
        if element is None:
            raise ValueError("Element ref expired; take a fresh snapshot of this tab")
        self.driver.call("POST", "/frame", {"id": None})
        if element["frame"] != "main":
            for index in element["frame"].removeprefix("main/").split("/"):
                self.driver.call("POST", "/frame", {"id": int(index)})
        return element["value"]

    def snapshot(self, tab_id, action):
        tab = self.switch(tab_id, action.get("frameID"))
        root = self.element(tab, action["ref"]) if action.get("ref") else None
        tab["refs"].clear()
        candidates = self.driver.script("""
            const result = []; let truncated = false;
            const maxDepth = arguments[1];
            function visit(element, depth) {
              if (depth > maxDepth || result.length >= 500) { truncated=true; return; }
              if (!element.getClientRects().length || getComputedStyle(element).visibility === 'hidden') return;
              const rect=element.getBoundingClientRect();
              result.push({element,depth,text:element.children.length ? '' : (element.innerText || '').slice(0,300),
                value:element.matches('input:not([type=password]),textarea') ? element.value.slice(0,300) : null,
                checked:element.matches('input[type=checkbox],input[type=radio]') ? element.checked : null,
                box:{x:rect.x,y:rect.y,width:rect.width,height:rect.height}});
              if (element.matches('input,textarea,[contenteditable=true]')) return;
              for (const child of element.children) visit(child,depth+1);
              if (element.shadowRoot) for (const child of element.shadowRoot.children) visit(child,depth+1);
            }
            visit(arguments[0] || document.body,0); return {result,truncated};
        """, root, action.get("depth", 8))
        lines = []
        frame = action.get("frameID", "main")
        for item in candidates["result"]:
            element = item["element"]
            path = "/element/" + element[ELEMENT]
            role = self.driver.call("GET", path + "/computedrole") or "generic"
            label = self.driver.call("GET", path + "/computedlabel")
            label = label or item["text"]
            if role == "generic" and not label:
                continue
            self.next_ref += 1
            ref = "e" + str(self.next_ref)
            tab["refs"][ref] = {"value": element, "frame": frame}
            line = "  " * item["depth"] + f"@{ref} [{role}] " + json.dumps(label[:300], ensure_ascii=False)
            if item["value"] is not None:
                line += " value=" + json.dumps(item["value"], ensure_ascii=False)
            if item["checked"] is not None:
                line += " checked=" + str(item["checked"]).lower()
            if action.get("boxes"):
                line += " box=" + json.dumps(item["box"])
            lines.append(line)
        if action["type"] == "find":
            lines = [line for line in lines if action["text"].lower() in line.lower()]
        content = "\n".join(lines)
        return {"tab": self.state(tab_id), "content": content[:100000],
                "truncated": candidates["truncated"] or len(content) > 100000}

    def execute(self, command):
        try:
            return self.dispatch(command)
        finally:
            if self.focused in self.tabs:
                self.switch(self.focused)

    def dispatch(self, command):
        action = command["action"]
        method = action["type"]
        if method not in OPERATIONS:
            raise ValueError(f"Default-browser adapter does not implement {method}")
        if method == "tabs.list":
            return {"value": self.inventory(), "files": []}
        if method == "tabs.open":
            self.inventory()
            handle = self.driver.call("POST", "/window/new", {"type": "tab"})["handle"]
            self.current_handle = None
            self.inventory()
            tab_id = next(key for key, tab in self.tabs.items() if tab["handle"] == handle)
            self.switch(tab_id)
            self.driver.call("POST", "/url", {"url": action.get("url", "about:blank")})
            if action.get("focus", True):
                self.focused = tab_id
            return {"value": self.state(tab_id), "files": []}
        tab_id = action["tabID"]
        state = self.state(tab_id)
        tab = self.switch(tab_id, action.get("frameID"))
        if command.get("inspect"):
            return {"value": {"resources": [action.get("url", state["url"])],
                              "key": str(state["generation"])}, "files": []}
        if command.get("target") and command["target"] != {
            "resources": [action.get("url", state["url"])], "key": str(state["generation"])
        }:
            raise ValueError("Target changed before execution; inspect the tab again")
        if command.get("generation") is not None and command["generation"] != state["generation"]:
            raise ValueError("Document changed before execution; refresh the tab and its snapshot")
        if method == "tabs.focus":
            self.focused = tab_id
        elif method == "tabs.close":
            if len(self.tabs) == 1:
                # WebDriver closes its session with the final window; keep a fresh empty tab instead.
                self.driver.call("POST", "/window/new", {"type": "tab"})
                self.switch(tab_id)
            self.driver.call("DELETE", "/window")
            self.current_handle = None
            return {"value": self.inventory(), "files": []}
        elif method in ("navigate", "back", "forward", "reload"):
            endpoint = {"navigate": "url", "reload": "refresh"}.get(method, method)
            self.driver.call("POST", "/" + endpoint, {"url": action["url"]} if method == "navigate" else {})
            if method == "back":
                tab["position"] = max(0, tab["position"] - 1)
            if method == "forward":
                tab["position"] = min(len(tab["history"]) - 1, tab["position"] + 1)
        elif method == "stop":
            self.driver.script("window.stop()")
        elif method in ("snapshot", "find"):
            return {"value": self.snapshot(tab_id, action), "files": []}
        elif method == "frames":
            frames = []
            def visit(frame_id, parent=None):
                self.switch(tab_id, frame_id)
                frames.append({"id": frame_id, "url": self.driver.script("return location.href"),
                               "name": self.driver.script("return window.name"),
                               **({"parentID": parent} if parent else {})})
                count = self.driver.script("return document.querySelectorAll('iframe,frame').length")
                for index in range(count):
                    visit(frame_id + "/" + str(index), frame_id)
            visit("main")
            return {"value": {"tab": self.state(tab_id), "frames": frames}, "files": []}
        elif method == "evaluate":
            element = self.element(tab, action["ref"]) if action.get("ref") else None
            value = self.driver.call("POST", "/execute/async", {"script": """
                const done=arguments[arguments.length-1];
                Promise.resolve().then(() => arguments[1] ? (0,eval)('('+arguments[0]+')')(arguments[1])
                  : (0,eval)(arguments[0])).then(value => done({value:value ?? null}),
                  error => done({error:String(error)}));
            """, "args": [action["script"], element]})
            if "error" in value:
                raise ValueError(value["error"])
            return {"value": {"tab": self.state(tab_id), "value": value["value"]}, "files": []}
        elif method == "fill_form":
            for field in action["fields"]:
                kind = {"text": "fill", "select": "select", "check": "check"}[field["type"]]
                field_action = {"type": kind, "tabID": tab_id, "ref": field["ref"]}
                field_action.update({"text": field["value"]} if kind == "fill" else
                                    {"values": field["values"]} if kind == "select" else {"checked": field["checked"]})
                self.execute({"action": field_action, "files": []})
        elif method in ("fill", "select", "check", "click", "hover", "drag", "files.upload", "files.drop"):
            element = self.element(tab, action.get("ref", action.get("from")))
            path = "/element/" + element[ELEMENT]
            if method == "fill":
                self.driver.call("POST", path + "/clear", {})
                self.driver.call("POST", path + "/value", {"text": action["text"]})
            elif method == "select":
                self.driver.script("""
                    const el=arguments[0],values=arguments[1];
                    if (!(el instanceof HTMLSelectElement) || el.disabled) throw Error('Not an enabled select');
                    if (!el.multiple && values.length !== 1) throw Error('Select requires one value');
                    for (const value of values) if (![...el.options].some(o => o.value===value && !o.disabled))
                      throw Error('Option missing or disabled');
                    for (const option of el.options) option.selected=values.includes(option.value);
                    el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));
                """, element, action["values"])
            elif method == "check":
                current = self.driver.call("GET", path + "/selected")
                if current != action["checked"]:
                    self.driver.call("POST", path + "/click", {})
                if self.driver.call("GET", path + "/selected") != action["checked"]:
                    raise ValueError("Page did not retain the requested checked state")
            elif method in ("files.upload", "files.drop"):
                files = command["files"]
                decoded = [base64.b64decode(file["data"], validate=True) for file in files]
                if not files or sum(map(len, decoded)) > MAX_FILE_BYTES:
                    raise ValueError("File transfer is empty or exceeds 5 MiB")
                if method == "files.upload":
                    paths = []
                    for index, (file, data) in enumerate(zip(files, decoded)):
                        path_on_disk = Path(self.driver.profile.name) / (str(index) + "-" + Path(file["name"]).name)
                        path_on_disk.write_bytes(data)
                        paths.append(str(path_on_disk))
                    self.driver.call("POST", path + "/value", {"text": "\n".join(paths)})
                else:
                    self.driver.script("""
                        const transfer=new DataTransfer();
                        for(const file of arguments[1]) transfer.items.add(new File(
                          [Uint8Array.from(atob(file.data),c=>c.charCodeAt(0))],file.name,{type:file.mime}));
                        for(const type of ['dragenter','dragover','drop']) arguments[0].dispatchEvent(
                          new DragEvent(type,{bubbles:true,cancelable:true,dataTransfer:transfer}));
                    """, element, files)
            else:
                pointer = [{"type": "pointerMove", "origin": element, "x": 0, "y": 0, "duration": 100}]
                if method == "drag":
                    pointer += [{"type": "pointerDown", "button": 0},
                                {"type": "pointerMove", "origin": self.element(tab, action["to"]), "x": 0, "y": 0, "duration": 500},
                                {"type": "pointerUp", "button": 0}]
                if method == "click":
                    button = {"left": 0, "middle": 1, "right": 2}[action.get("button", "left")]
                    pointer += [{"type": kind, "button": button} for _ in range(action.get("count", 1))
                                for kind in ("pointerDown", "pointerUp")]
                modifiers = [KEYS[key] for key in action.get("modifiers", [])]
                sources = [{"type": "pointer", "id": "mouse", "parameters": {"pointerType": "mouse"}, "actions": pointer}]
                if modifiers:
                    sources.append({"type": "key", "id": "keyboard", "actions":
                                    [{"type": "keyDown", "value": key} for key in modifiers]})
                self.driver.call("POST", "/actions", {"actions": sources})
                self.driver.call("DELETE", "/actions")
        elif method == "press":
            parts = action["key"].split("+")
            keys = [KEYS.get(part, part) for part in parts]
            if not all(len(key) == 1 for key in keys):
                raise ValueError("Unsupported key chord")
            self.driver.call("POST", "/actions", {"actions": [{"type": "key", "id": "keyboard", "actions":
                [{"type": "keyDown", "value": key} for key in keys] +
                [{"type": "keyUp", "value": key} for key in reversed(keys)]}]})
        elif method == "scroll":
            self.driver.script("window.scrollBy(arguments[0],arguments[1])", action.get("deltaX", 0), action["deltaY"])
        elif method == "wait":
            deadline = time.monotonic() + action.get("timeoutMs", 10000) / 1000
            while time.monotonic() < deadline:
                value = self.driver.script("return document.readyState === 'complete'" if action["condition"] == "load"
                                           else "return document.body.innerText.includes(arguments[0])", action.get("text", ""))
                if value == (action["condition"] != "textGone"):
                    break
                time.sleep(0.05)
            else:
                raise TimeoutError("Wait condition did not become true; inspect the current page")
        elif method == "dialog":
            try:
                message = self.driver.call("GET", "/alert/text")
            except RuntimeError as error:
                if "no such alert" not in str(error):
                    raise
                message = None
            if action["action"] != "get":
                if message is None:
                    raise ValueError("No JavaScript dialog is open")
                if action.get("promptText") is not None:
                    self.driver.call("POST", "/alert/text", {"text": action["promptText"]})
                self.driver.call("POST", "/alert/" + action["action"], {})
                message = None
            return {"value": {"tab": self.state(tab_id), "dialog": None if message is None else
                              {"type": "dialog", "message": message[:100000], "defaultValue": ""}}, "files": []}
        elif method == "screenshot":
            from PIL import Image
            if action.get("ref") and action.get("fullPage"):
                raise ValueError("Choose an element or fullPage, not both")
            if action.get("ref"):
                element = self.element(tab, action["ref"])
                raw = self.driver.call("GET", "/element/" + element[ELEMENT] + "/screenshot")
            elif action.get("fullPage") and self.driver.browser["family"] == "firefox":
                raw = self.driver.call("GET", "/moz/screenshot/full")
            elif action.get("fullPage"):
                raw = self.driver.call("POST", "/goog/cdp/execute", {"cmd": "Page.captureScreenshot",
                    "params": {"captureBeyondViewport": True, "fromSurface": True}})["data"]
            else:
                raw = self.driver.call("GET", "/screenshot")
            image = Image.open(io.BytesIO(base64.b64decode(raw)))
            if image.width * image.height > 16_000_000:
                raise ValueError("Capture exceeds 16 megapixels; choose an element or viewport")
            if image.width > action.get("maxWidth", 2000):
                image.thumbnail((action.get("maxWidth", 2000), image.height))
            output = io.BytesIO()
            image_format = action.get("format", "png")
            image.convert("RGB" if image_format == "jpeg" else "RGBA").save(
                output, format={"png": "PNG", "jpeg": "JPEG", "webp": "WEBP"}[image_format],
                **({"quality": action.get("quality", 80)} if image_format != "png" else {}))
            data = output.getvalue()
            if len(data) > MAX_FILE_BYTES:
                raise ValueError("Capture exceeds 5 MiB; reduce maxWidth or quality")
            file = {"id": "file_" + str(uuid.uuid4()), "name": "screenshot." + image_format,
                    "mime": "image/" + image_format, "data": base64.b64encode(data).decode()}
            tab["files"][file["id"]] = file
            return {"value": {"tab": self.state(tab_id)}, "files": [file]}
        elif method == "files.list":
            return {"value": {"tab": self.state(tab_id), "files": [
                {"id": file["id"], "name": file["name"], "mime": file["mime"],
                 "bytes": len(base64.b64decode(file["data"])), "state": "completed"}
                for file in tab["files"].values()]}, "files": []}
        elif method == "files.get":
            if action["fileID"] not in tab["files"]:
                raise ValueError("File is not owned by this tab")
            return {"value": {"tab": self.state(tab_id)}, "files": [tab["files"][action["fileID"]]]}
        return {"value": self.state(tab_id), "files": []}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True)
    options = parser.parse_args()
    def stop(_signal, _frame):
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    driver = WebDriver(json.loads(Path(options.config).read_text()))
    try:
        host = Host(driver)
        host.inventory()
        print(json.dumps({"type": "ready", "version": 1, "operations": OPERATIONS}), flush=True)
        while line := sys.stdin.buffer.readline(MAX_LINE_BYTES + 1):
            if len(line) > MAX_LINE_BYTES or not line.endswith(b"\n"):
                raise ValueError("Host command exceeds the protocol bound")
            request = json.loads(line)
            try:
                result = host.execute(request["command"])
                response = {"id": request["id"], "outcome": {"type": "success", "result": result}}
            except (ValueError, RuntimeError, OSError, TimeoutError) as error:
                response = {"id": request["id"], "outcome": {"type": "failure", "code": "adapter", "message": str(error)[:2048]}}
            encoded = json.dumps(response)
            if len(encoded.encode()) > MAX_LINE_BYTES:
                raise ValueError("Host response exceeds the protocol bound")
            print(encoded, flush=True)
    finally:
        driver.close()


if __name__ == "__main__":
    main()
