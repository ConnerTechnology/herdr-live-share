// Live share updates: the version and two buttons. "Update mods and engines"
// installs a new release (the mods here and the engine on every machine), or,
// with the mods current, brings any engine that is missing or behind up to
// them; with nothing to update it is greyed out. "What's new" opens the release
// notes. Opening the dialog asks GitHub right away (there is no Check now).
// Nothing installs without a click.
import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { props, closeOverlay, theme, checking, machines } from "herdr";
import { engine, installEngine, checkUpdate, updateLine, updateAll, newer, machineName, releaseUrl, openUrl, VERSION } from "./engine.js";

export default class ShareUpdates extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.handle.focus();
    this.line = "Checking…";
    this.update = null;
    this.busy = null; // what is running: "check" or "update"
    this.engines = new Map(); // machine key → { on, text, fix }
    if (checking()) return;
    cx.spawn(async (cx) => {
      await Promise.all([this.checkForUpdate(cx, true), this.checkEngines(cx)]);
    });
  }

  async checkForUpdate(cx, force) {
    this.busy = "check";
    this.line = "Checking…";
    cx.notify();
    const r = await checkUpdate(cx, { force });
    this.busy = null;
    this.update = r.update;
    this.latest = r.latest;
    this.line = updateLine(r);
    cx.notify();
  }

  async checkEngines(cx) {
    const list = machines().filter((m) => m.layout !== "unavailable");
    const names = [null, ...list.map((m) => m.name).filter((n) => n !== null)];
    for (const on of names) this.engines.set(on ?? "", { on, text: "Checking…", fix: null });
    cx.notify();
    await Promise.all(names.map((on) => this.checkEngine(cx, on)));
  }

  async checkEngine(cx, on) {
    const r = await engine(["version"], { on });
    let row;
    if (r.ok && newer(VERSION, r.value.version)) row = { on, text: `herdr-session ${r.value.version}, older than the mods (${VERSION}).`, fix: "update" };
    else if (r.ok) row = { on, text: `herdr-session ${r.value.version}. Up to date.`, fix: null };
    else row = { on, text: r.error.message, fix: r.error.kind === "Missing" ? "install" : null };
    this.engines.set(on ?? "", row);
    cx.notify();
  }

  /** Engines missing or behind, on every machine at once (the mods are current). */
  updateEngines(cx) {
    const behind = [...this.engines.values()].filter((e) => e.fix);
    this.busy = "update";
    this.line = "Updating the engine on every machine…";
    cx.notify();
    cx.spawn(async (cx) => {
      await Promise.all(behind.map(async (e) => {
        const r = await installEngine(e.on);
        if (r.ok) await this.checkEngine(cx, e.on);
        else { this.engines.set(e.on ?? "", { on: e.on, text: r.error.message, fix: e.fix }); cx.notify(); }
      }));
      this.busy = null;
      const left = [...this.engines.values()].filter((e) => e.fix);
      this.line = left.length ? left.map((e) => `${machineName(e.on)}: ${e.text}`).join(" ") : "Up to date.";
      cx.notify();
    });
  }

  /** The line under the version: the release check, or which engines Update engines will fix. */
  statusLine() {
    const behind = [...this.engines.values()].filter((e) => e.fix);
    if (this.update || this.busy || !behind.length) return this.line;
    const where = behind.map((e) => machineName(e.on)).join(", ");
    return `Up to date here. The engine on ${where} ${behind.some((e) => e.fix === "install") ? "needs installing" : "is behind"}.`;
  }

  updateEverything(cx) {
    this.busy = "update";
    this.line = `Updating to ${this.update}: the mods here and the engine on every machine…`;
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await updateAll();
      // On success the mods are replaced and this dialog reloads as the new version.
      this.busy = null;
      this.line = r.ok ? `Updated to ${this.update}.` : r.error.message;
      cx.notify();
      await this.checkEngines(cx);
    });
  }

  close() {
    closeOverlay(props()?.overlay);
  }

  render(cx) {
    const t = theme();
    const idle = this.busy === null;
    const button = (id, label, onClick, primary, enabled) =>
      h_flex().id(id).h(36).px(16).items_center().rounded(8).border_1()
        .border_color(primary ? t.accent : t.border).bg(primary ? t.accent : t.popover)
        .text_size(13).font_medium().text_color(primary ? t.accent_foreground : t.foreground)
        .when(!enabled, (el) => el.opacity(0.45))
        .when(enabled, (el) => el.cursor_pointer().hover((s) => s.opacity(0.88)).on_click(onClick))
        .child(label);

    // One button for the mods and every engine: a new release, else engines that fell behind.
    const enginesBehind = [...this.engines.values()].some((e) => e.fix);
    const canUpdate = idle && (Boolean(this.update) || enginesBehind);
    const update = (_e, cx) => (this.update ? this.updateEverything(cx) : this.updateEngines(cx));
    const notes = releaseUrl(this.update ?? VERSION);

    return v_flex()
      .id("share-updates")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key !== "escape") return;
        this.close();
        cx.stop_propagation();
      })
      .size_full()
      .p(24)
      .gap(16)
      .rounded(8)
      .bg(t.popover)
      .child(h_flex().items_center()
        .child(div().flex_1().text_size(17).font_semibold().text_color(t.foreground).child("Live share updates"))
        .child(div().text_size(11).text_color(t.dim).child("Esc")))
      .child(v_flex().gap(4)
        .child(div().text_size(14).font_semibold().text_color(t.foreground).child(`herdr live share ${VERSION}`))
        .child(div().text_size(12).text_color(t.muted).child(this.statusLine())))
      .child(h_flex().gap(8)
        .child(button("update", "Update mods and engines", update, true, canUpdate))
        .child(button("whats-new", "What's new", () => openUrl(notes), false, true)));
  }
}
