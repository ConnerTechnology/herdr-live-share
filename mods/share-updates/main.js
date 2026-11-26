// Live share updates: like Herdr's Settings → Updates. The mods on this
// computer (What's new, Check now, Update) and the engine on every machine.
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
    this.busy = null; // what is running: "check", "update", or a machine key
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

  fixEngine(cx, on) {
    this.busy = on ?? "";
    this.engines.set(on ?? "", { on, text: `Installing herdr-session on ${machineName(on)}…`, fix: null });
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await installEngine(on);
      this.busy = null;
      if (r.ok) await this.checkEngine(cx, on);
      else { this.engines.set(on ?? "", { on, text: r.error.message, fix: "install" }); cx.notify(); }
    });
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
      h_flex().id(id).gap(12).items_center().py(12).border_b_1().border_color(t.border)
        .child(v_flex().flex_1().gap(4)
          .child(div().text_size(14).text_color(t.foreground).child(title))
          .child(div().text_size(12).text_color(t.muted).child(detail)))
        .child(h_flex().gap(8).children(actions));

    const modsActions = this.update
      ? [button("whats-new", "What's new", () => openUrl(releaseUrl(this.update))), button("update", `Update to ${this.update}`, (_e, cx) => this.updateEverything(cx), true)]
      : [button("whats-new", "What's new", () => openUrl(releaseUrl(VERSION))), button("check", "Check now", (_e, cx) => cx.spawn(async (cx) => this.checkForUpdate(cx, true)))];

    const engineRows = [...this.engines.entries()].map(([key, e]) =>
      row(`engine-${key}`, machineName(e.on), e.text,
        e.fix && idle ? [button(`fix-${key}`, e.fix === "install" ? `Install on ${machineName(e.on)}` : `Update on ${machineName(e.on)}`, (_e, cx) => this.fixEngine(cx, e.on))] : []));

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
        .child(div().flex_1().text_size(18).font_semibold().text_color(t.foreground).child("Live share updates"))
        .child(div().text_size(11).text_color(t.dim).child("Esc")))
      .child(section("HERDR LIVE SHARE"))
      .child(row("mods", `herdr live share ${VERSION}`, this.line, idle ? modsActions : []))
      .child(div().h(8))
      .child(section("ENGINE ON EACH MACHINE"))
      .children(engineRows)
      .child(div().pt(8).text_size(12).text_color(t.dim).child(
        "Share dialogs look for a new release each time they open. Updates install only when you click Update: the mods here, then the engine on every connected machine.",
      ));
  }
}
