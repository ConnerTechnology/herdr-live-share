// Calls the herdr-session engine. Byte-identical copy in every share-* mod
// (mods cannot import each other); scripts/check verifies the copies match.
//
// The engine runs on the machine that holds the space's repo, which is often
// not the machine running herdr-app: every call goes through exec's `on`.
import { exec, machines, spaces } from "herdr";

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
