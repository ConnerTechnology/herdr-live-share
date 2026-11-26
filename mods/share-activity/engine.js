// Calls the herdr-session engine. Byte-identical copy in every share-* mod
// (mods cannot import each other); scripts/check verifies the copies match.
//
// The engine runs on the machine that holds the space's repo, which is often
// not the machine running herdr-app: every call goes through exec's `on`.
import { exec, machines, spaces, shared, setShared } from "herdr";

export const API = 1;

/** The shown space, or null when none is shown. */
export function shownSpace() {
  return spaces().find((s) => s.active) ?? null;
}

/** A machine's display name; null is the computer running herdr-app. */
export function machineName(on) {
  return on ?? "this computer";
}

/** Run one engine verb on a machine (a space's `machine`; null: this computer). Never throws.
 *  -> { ok: true, value } | { ok: false, error: { kind, message } }
 *  kind: "Unreachable" (machine offline), "Missing" (not on that machine's exec PATH),
 *  "Version" (api mismatch), "Broken" (nonzero exit or bad JSON), or an engine error kind. */
export async function engine(args, { on = null, cwd, timeout = 30_000 } = {}) {
  const where = machineName(on);
  if (on && machines().find((m) => m.name === on)?.layout === "unavailable") {
    return { ok: false, error: { kind: "Unreachable", message: `Can't reach ${where}.` } };
  }
  let out;
  try {
    out = await exec("herdr-session", args, { on, cwd, timeout });
  } catch {
    return { ok: false, error: { kind: "Missing", message: `herdr-session isn't installed on ${where}.` } };
  }
  if (out.code !== 0) return { ok: false, error: { kind: "Broken", message: (out.stderr || out.stdout || "").trim() } };
  let env;
  try {
    env = JSON.parse(out.stdout);
  } catch {
    return { ok: false, error: { kind: "Broken", message: `herdr-session on ${where} printed something that is not JSON.` } };
  }
  if (env.api !== API) {
    return { ok: false, error: { kind: "Version", message: `herdr-session on ${where} speaks api ${env.api}; these mods need ${API}. Update both.` } };
  }
  return env.ok ? { ok: true, value: env.value } : { ok: false, error: env.error };
}

const INSTALL = "https://raw.githubusercontent.com/ConnerTechnology/herdr-live-share/main/install.sh";

/** Install the engine on a machine (null: this computer) with the same one-liner people run.
 *  Builds from source when no release binary fits, so it can take minutes. Never throws. */
export async function installEngine(on = null) {
  try {
    const out = await exec("sh", ["-c", `curl -fsSL '${INSTALL}' | sh -s -- --engine`], { on, timeout: 600_000 });
    if (out.code === 0) return { ok: true };
    return { ok: false, error: { kind: "Install", message: (out.stderr || out.stdout || "").trim().split("\n").slice(-2).join(" ") } };
  } catch {
    return { ok: false, error: { kind: "Unreachable", message: `Can't reach ${machineName(on)}.` } };
  }
}

/** This release of the mods. scripts/release keeps it equal to engine/Cargo.toml. */
export const VERSION = "0.1.6";

const RELEASES = "https://api.github.com/repos/ConnerTechnology/herdr-live-share/releases/latest";
const RECHECK = 10 * 60_000;

/** a > b for "x.y.z" versions. */
export function newer(a, b) {
  const pa = a.split(".").map(Number), pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) > (pb[i] ?? 0);
  return false;
}

/** The update check: { latest, update, checkedAt, via, error }.
 *  latest: the newest release ("0.2.0") or null; update: latest when it is newer
 *  than these mods. Asks GitHub at most once per 10 minutes unless `force`
 *  (Check now); the last success is cached in shared("share.latest") for all
 *  share mods. Tries herdr-app's fetch (capped at 8 s), then curl. Never throws. */
export async function checkUpdate(cx, { force = false } = {}) {
  let latest = shared("share.latest");
  let via = null, error = null;
  if (force || !latest || Date.now() - latest.at > RECHECK) {
    const r = await latestTag(cx);
    if (r.tag) {
      latest = { tag: r.tag, at: Date.now() };
      setShared("share.latest", latest);
      via = r.via;
    } else {
      error = r.error;
    }
  }
  const tag = latest?.tag ?? null;
  return { latest: tag, update: tag && newer(tag, VERSION) ? tag : null, checkedAt: latest?.at ?? null, via, error };
}

/** One line for a dialog: "Up to date. Checked at 1:24." and the like. */
export function updateLine(r) {
  if (r.error && !r.latest) return `Could not check for updates: ${r.error}`;
  const when = r.checkedAt ? ` Checked at ${clock(r.checkedAt)}.` : "";
  if (r.update) return `Version ${r.update} is available.${when}`;
  return `Up to date.${when}`;
}

/** "1:24" in local time. */
export function clock(ms) {
  const d = new Date(ms);
  return `${d.getHours() % 12 || 12}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** The GitHub page for a release's notes ("0.1.4"), or all releases. */
export function releaseUrl(tag) {
  const base = "https://github.com/ConnerTechnology/herdr-live-share/releases";
  return tag ? `${base}/tag/v${tag}` : base;
}

/** Open a URL in the browser on the computer running herdr-app. */
export async function openUrl(url) {
  for (const opener of ["open", "xdg-open"]) {
    try {
      const out = await exec(opener, [url], { timeout: 10_000 });
      if (out.code === 0) return true;
    } catch {}
  }
  return false;
}

async function latestTag(cx) {
  const parse = (text) => String(JSON.parse(text).tag_name ?? "").replace(/^v/, "") || null;
  const errors = [];
  try {
    const timeout = cx.sleep(8_000).then(() => { throw new Error("timed out"); });
    const r = await Promise.race([fetch(RELEASES, { headers: { accept: "application/vnd.github+json", "user-agent": "herdr-live-share" } }), timeout]);
    if (r.ok) {
      const tag = parse(await r.text());
      if (tag) return { tag, via: "fetch" };
    }
    errors.push(`fetch ${r.status}`);
  } catch (e) {
    errors.push(`fetch ${e?.message ?? e}`);
  }
  try {
    const out = await exec("curl", ["-fsSL", "-H", "accept: application/vnd.github+json", RELEASES], { timeout: 15_000 });
    if (out.code === 0) {
      const tag = parse(out.stdout);
      if (tag) return { tag, via: "curl" };
    }
    errors.push(`curl exit ${out.code} ${(out.stderr || "").trim().slice(0, 80)}`);
  } catch (e) {
    errors.push(`curl ${e?.message ?? e}`);
  }
  return { error: errors.join("; ") };
}

/** Update everything from the computer running herdr-app: the mods here (they hot-reload)
 *  and the engine on every connected machine. Runs only when the person clicks Update. Never throws. */
export async function updateAll() {
  try {
    const out = await exec("sh", ["-c", `curl -fsSL '${INSTALL}' | sh`], { timeout: 600_000 });
    if (out.code === 0) return { ok: true };
    return { ok: false, error: { kind: "Install", message: (out.stderr || out.stdout || "").trim().split("\n").slice(-2).join(" ") } };
  } catch {
    return { ok: false, error: { kind: "Install", message: "Could not run the installer on this computer." } };
  }
}
