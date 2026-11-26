// Activity: placeholder view (scaffold). The real screen is specified in
// SPEC.md section 5.2; this one proves the palette entry, overlay, engine call,
// install and update work end to end before any logic exists.
import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { props, closeOverlay, theme, checking } from "herdr";
import { engine, installEngine, availableUpdate, updateAll, newer, shownSpace, machineName, VERSION } from "./engine.js";

export default class Activity extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.handle.focus();
    this.engine = "Checking the engine…";
    this.fix = null;      // "install" | "update-engine": the engine on the space's machine needs it
    this.update = null;   // a newer release, offered and never installed without a click
    this.busy = false;
    if (checking()) return;
    cx.spawn(async (cx) => {
      await this.check(cx);
      this.update = await availableUpdate();
      cx.notify();
    });
  }

  async check(cx) {
    this.on = shownSpace()?.machine ?? null;
    const r = await engine(["version"], { on: this.on });
    const where = machineName(this.on);
    if (r.ok && newer(VERSION, r.value.version)) {
      this.fix = "update-engine";
      this.engine = `herdr-session ${r.value.version} on ${where} is older than these mods (${VERSION}).`;
    } else if (r.ok) {
      this.fix = null;
      this.engine = `herdr-session ${r.value.version} (api ${r.value.api}) on ${where}.`;
    } else {
      this.fix = r.error.kind === "Missing" ? "install" : null;
      this.engine = r.error.message;
    }
    cx.notify();
  }

  /** Engine missing or older on the space's machine: install it there, then check again. */
  installHere(cx) {
    this.busy = true;
    this.engine = `Installing herdr-session on ${machineName(this.on)}…`;
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await installEngine(this.on);
      this.busy = false;
      if (r.ok) await this.check(cx);
      else { this.engine = r.error.message; cx.notify(); }
    });
  }

  /** A newer release: update the mods here and the engine everywhere. The mods hot-reload when it lands. */
  updateEverything(cx) {
    this.busy = true;
    this.engine = `Updating to ${this.update}…`;
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await updateAll();
      this.busy = false;
      this.engine = r.ok ? `Updated to ${this.update}.` : r.error.message;
      cx.notify();
    });
  }

  close() {
    closeOverlay(props()?.overlay);
  }

  render(cx) {
    const t = theme();
    const button = (id, label, primary, onClick) =>
      h_flex().id(id).h(32).px(14).items_center().rounded(8)
        .bg(primary ? t.solid : t.raised).text_color(primary ? t.solid_foreground : t.foreground)
        .cursor_pointer().hover((s) => s.opacity(0.88)).on_click(onClick).child(label);
    const fixLabel = this.fix === "install" ? `Install on ${machineName(this.on)}` : `Update on ${machineName(this.on)}`;
    return v_flex()
      .id("share-activity")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key !== "escape") return;
        this.close();
        cx.stop_propagation();
      })
      .size_full()
      .p(18)
      .gap(12)
      .rounded(8)
      .bg(t.popover)
      .text_size(13)
      .child(div().text_size(15).font_semibold().text_color(t.foreground).child("Activity"))
      .child(div().text_color(t.muted).child("The session timeline: joins, pushes, tasks, and what each agent is working on."))
      .child(div().text_size(11).text_color(t.dim).child(`Not built yet. ${this.engine}`))
      .when(Boolean(this.update) && !this.busy, (el) => el.child(div().text_size(11).text_color(t.dim).child(`Version ${this.update} is available (you have ${VERSION}).`)))
      .child(
        h_flex().justify_end().gap(8)
          .when(Boolean(this.update) && !this.busy, (el) => el.child(button("update", `Update to ${this.update}`, true, (_e, cx) => this.updateEverything(cx))))
          .when(Boolean(this.fix) && !this.update && !this.busy, (el) => el.child(button("fix", fixLabel, true, (_e, cx) => this.installHere(cx))))
          .child(button("close", "Close", false, () => this.close())),
      );
  }
}
