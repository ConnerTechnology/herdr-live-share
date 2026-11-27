// Live share updates: like Herdr's Settings → Updates. One row for herdr live
// share and one button that updates everything: Update to … (the mods here and
// the engine on every machine), or Update engines when only an engine is
// behind. Engines are checked in the background only to pick that label; no
// rows per machine. Nothing installs without a click.
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
      await Promise.all([this.checkForUpdate(cx, false), this.checkEngines(cx)]);
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
    const button = (id, label, onClick, primary = false) =>
      h_flex().id(id).h(36).px(16).items_center().rounded(8).border_1()
        .border_color(primary ? t.solid : t.border).bg(primary ? t.solid : t.popover)
        .text_color(primary ? t.solid_foreground : t.foreground)
        .cursor_pointer().hover((s) => s.opacity(0.88)).on_click(onClick).child(label);
    const section = (label) => div().text_size(12).text_color(t.dim).child(label);
    const row = (id, title, detail, actions) =>
      h_flex().id(id).gap(12).items_center().py(12)
        .child(v_flex().flex_1().gap(4)
          .child(div().text_size(14).text_color(t.foreground).child(title))
          .child(div().text_size(12).text_color(t.muted).child(detail)))
        .child(h_flex().gap(8).children(actions));

    // One button does everything: a new release updates the mods and every engine;
    // with the mods current, it brings any engine that is missing or behind up to them.
    const enginesBehind = [...this.engines.values()].some((e) => e.fix);
    const modsActions = this.update
      ? [button("whats-new", "What's new", () => openUrl(releaseUrl(this.update))), button("update", `Update to ${this.update}`, (_e, cx) => this.updateEverything(cx), true)]
      : enginesBehind
        ? [button("whats-new", "What's new", () => openUrl(releaseUrl(VERSION))), button("update-engines", "Update engines", (_e, cx) => this.updateEngines(cx), true)]
        : [button("whats-new", "What's new", () => openUrl(releaseUrl(VERSION))), button("check", "Check now", (_e, cx) => cx.spawn(async (cx) => this.checkForUpdate(cx, true)))];

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
      .gap(8)
      .rounded(8)
      .bg(t.popover)
      .child(h_flex().items_center()
        .child(div().flex_1().text_size(17).font_semibold().text_color(t.foreground).child("Live share updates"))
        .child(div().text_size(11).text_color(t.dim).child("Esc")))
      .child(section("HERDR LIVE SHARE"))
      .child(row("mods", `herdr live share ${VERSION}`, this.statusLine(), idle ? modsActions : []));
  }
}
