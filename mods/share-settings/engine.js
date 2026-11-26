// Calls the herdr-session engine. Byte-identical copy in every share-* mod
// (mods cannot import each other); scripts/check verifies the copies match.
import { exec } from "herdr";

export const API = 1;

/** Run one engine verb. Never throws.
 *  -> { ok: true, value } | { ok: false, error: { kind, message } }
 *  kind: "Missing" (binary not on PATH), "Version" (api mismatch),
 *  "Broken" (nonzero exit or bad JSON), or an engine error kind. */
export async function engine(args, { on, timeout = 30_000, cwd } = {}) {
  let out;
  try {
    out = await exec("herdr-session", args, { on, timeout, cwd });
  } catch (e) {
    return { ok: false, error: { kind: "Missing", message: "herdr-session is not installed or not on PATH." } };
  }
  if (out.code !== 0) return { ok: false, error: { kind: "Broken", message: (out.stderr || out.stdout || "").trim() } };
  let env;
  try {
    env = JSON.parse(out.stdout);
  } catch {
    return { ok: false, error: { kind: "Broken", message: "herdr-session printed something that is not JSON." } };
  }
  if (env.api !== API) return { ok: false, error: { kind: "Version", message: `herdr-session speaks api ${env.api}, this mod needs ${API}.` } };
  return env.ok ? { ok: true, value: env.value } : { ok: false, error: env.error };
}
