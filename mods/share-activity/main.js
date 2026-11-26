// Activity: placeholder view (scaffold). The real screen is specified in
// SPEC.md section 5.2; this one proves the palette entry, overlay and engine
// call work end to end before any logic exists.
import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { props, closeOverlay, theme, checking } from "herdr";
import { engine, installEngine, shownSpace, machineName } from "./engine.js";

export default class Activity extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.handle.focus();
    this.engine = "Checking the engine…";
    if (checking()) return;
    cx.spawn(async (cx) => {
      await this.check(cx);
    });
  }

  async check(cx) {
    this.on = shownSpace()?.machine ?? null;
    const r = await engine(["version"], { on: this.on });
    this.missing = !r.ok && r.error.kind === "Missing";
    this.engine = r.ok ? `herdr-session ${r.value.version} (api ${r.value.api}) on ${machineName(this.on)}.` : r.error.message;
    cx.notify();
  }

  /** Engine missing on the space's machine: install it there, then check again. */
  install(cx) {
    this.installing = true;
    this.engine = `Installing herdr-session on ${machineName(this.on)}…`;
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await installEngine(this.on);
      this.installing = false;
      if (r.ok) await this.check(cx);
      else { this.engine = r.error.message; cx.notify(); }
    });
  }

  close() {
    closeOverlay(props()?.overlay);
  }

  render(cx) {
    const t = theme();
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
      .child(
        h_flex().justify_end().gap(8)
          .when(this.missing && !this.installing, (el) => el.child(
            h_flex().id("install").h(32).px(14).items_center().rounded(8).bg(t.solid).text_color(t.solid_foreground)
              .cursor_pointer().hover((s) => s.opacity(0.88)).on_click((_e, cx) => this.install(cx))
              .child(`Install on ${machineName(this.on)}`),
          ))
          .child(
          h_flex().id("close").h(32).px(14).items_center().rounded(8).bg(t.raised).text_color(t.foreground)
            .cursor_pointer().hover((s) => s.opacity(0.88)).on_click(() => this.close()).child("Close"),
        ),
      );
  }
}
