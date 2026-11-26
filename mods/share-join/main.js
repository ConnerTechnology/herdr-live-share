// Join a space: placeholder view (scaffold). The real screen is specified in
// SPEC.md section 5.2; this one proves the palette entry, overlay and engine
// call work end to end before any logic exists.
import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { props, closeOverlay, theme, checking } from "herdr";
import { engine } from "./engine.js";

export default class ShareJoin extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.handle.focus();
    this.engine = "Checking the engine…";
    if (checking()) return;
    cx.spawn(async (cx) => {
      const r = await engine(["version"]);
      this.engine = r.ok ? `herdr-session ${r.value.version} (api ${r.value.api})` : r.error.message;
      cx.notify();
    });
  }

  close() {
    closeOverlay(props()?.overlay);
  }

  render(cx) {
    const t = theme();
    return v_flex()
      .id("share-join")
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
      .child(div().text_size(15).font_semibold().text_color(t.foreground).child("Join a space"))
      .child(div().text_color(t.muted).child("Paste a share link. Five checks run: session, GitHub sign-in, repo access, folder, branch."))
      .child(div().text_size(11).text_color(t.dim).child(`Not built yet. ${this.engine}`))
      .child(
        h_flex().justify_end().child(
          h_flex().id("close").h(32).px(14).items_center().rounded(8).bg(t.raised).text_color(t.foreground)
            .cursor_pointer().hover((s) => s.opacity(0.88)).on_click(() => this.close()).child("Close"),
        ),
      );
  }
}
