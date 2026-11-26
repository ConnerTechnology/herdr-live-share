// Share this space: placeholder view (scaffold). The real screen is specified in
// SPEC.md section 5.2; this one proves the palette entry, overlay, engine call,
// install and update work end to end before any logic exists.
import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { props, closeOverlay, theme, checking, run } from "herdr";
import { engine, installEngine, checkUpdate, newer, shownSpace, machineName, VERSION } from "./engine.js";

export default class ShareStart extends View {
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
      await this.checkForUpdate(cx, false);
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

  /** Ask GitHub for a newer release; `force` is Check now. */
  async checkForUpdate(cx, force) {
    this.checking = true;
    cx.notify();
    const r = await checkUpdate(cx, { force });
    this.checking = false;
    this.update = r.update;
    this.updateNote = r.update ? `Version ${r.update} is available` : "Up to date";
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

  /** The footer link: Live share updates owns checking and installing. */
  openUpdates() {
    this.close();
    run("mod.open", { name: "share-updates" });
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
      .id("share-start")
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
      .child(div().text_size(15).font_semibold().text_color(t.foreground).child("Share this space"))
      .child(div().text_color(t.muted).child("Step 1: commit and push your work. Step 2: name the space and start."))
      .child(div().text_size(11).text_color(t.dim).child(`Not built yet. ${this.engine}`))
      .child(
        // Footer: the version line on the left, the buttons on the right, one row.
        h_flex().items_center().gap(8)
          .child(
            h_flex().flex_1().gap(6).items_center().text_size(11)
              .when(Boolean(this.update), (el) => el.child(div().w(6).h(6).rounded(3).bg(t.accent)))
              .when(Boolean(this.updateNote), (el) => el
                .child(div().text_color(t.muted).child(`herdr live share ${VERSION} · ${this.updateNote}`))
                .child(div().id("updates").cursor_pointer().on_click(() => this.openUpdates()).underline().font_semibold().text_color(this.update ? t.accent : t.foreground).hover((s) => s.opacity(0.8))
                  .child(this.update ? "See update" : "Check for updates"))),
          )
          .when(Boolean(this.fix) && !this.busy, (el) => el.child(button("fix", fixLabel, true, (_e, cx) => this.installHere(cx))))
          .child(button("close", "Close", false, () => this.close())),
      );
  }
}
