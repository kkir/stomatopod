use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ButtonVariant {
    Primary,
    Danger,
}

/// Port of `.btn` + `.btn-primary`/`.btn-danger` (dashboard.css Buttons
/// section). Ghost-styled controls use the raw `BTN_GHOST` class instead
/// (they need `type="submit"` or are `<a>` links, which this component
/// cannot express).
#[component]
pub fn Button(
    variant: ButtonVariant,
    onclick: Option<EventHandler<MouseEvent>>,
    children: Element,
) -> Element {
    let variant_class = match variant {
        ButtonVariant::Primary => "bg-grad-btn text-[#032621] shadow-glow hover:-translate-y-px",
        ButtonVariant::Danger => "bg-red text-white shadow-sm hover:-translate-y-px",
    };
    rsx! {
        button {
            class: "inline-flex items-center gap-1.5 px-[15px] py-2 rounded-[10px] text-[13px] font-semibold tracking-tight cursor-pointer transition-all duration-150 {variant_class}",
            r#type: "button",
            onclick: move |evt| {
                if let Some(handler) = &onclick {
                    handler.call(evt);
                }
            },
            {children}
        }
    }
}
