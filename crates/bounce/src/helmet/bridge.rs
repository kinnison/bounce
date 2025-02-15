use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use gloo::utils::head;
use wasm_bindgen::UnwrapThrowExt;
use web_sys::Element;
use yew::prelude::*;
use yew::virtual_dom::AttrValue;

use super::state::{merge_helmet_states, HelmetState, HelmetTag};
use super::FormatTitle;
#[cfg(feature = "ssr")]
use super::StaticWriter;
use crate::states::artifact::use_artifacts;

#[cfg(debug_assertions)]
mod guard {
    use super::*;

    use std::rc::Rc;

    use crate::root_state::BounceRootState;
    use crate::states::slice::use_slice;
    use crate::Slice;

    enum HelmetBridgeGuardAction {
        Increment,
        Decrement,
    }

    /// A Guard to prevent multiple bridges to be registered.
    #[derive(Default, PartialEq, Slice)]
    struct HelmetBridgeGuard {
        inner: usize,
    }

    impl Reducible for HelmetBridgeGuard {
        type Action = HelmetBridgeGuardAction;

        fn reduce(self: Rc<Self>, action: Self::Action) -> Rc<Self> {
            match action {
                Self::Action::Increment => {
                    debug_assert_eq!(
                        self.inner, 0,
                        "attempts to register more than 1 helmet bridge."
                    );

                    Self {
                        inner: self.inner + 1,
                    }
                    .into()
                }
                Self::Action::Decrement => Self {
                    inner: self.inner - 1,
                }
                .into(),
            }
        }
    }

    #[hook]
    pub(super) fn use_helmet_guard() {
        let guard = use_slice::<HelmetBridgeGuard>();
        let root = use_context::<BounceRootState>().expect_throw("No bounce root found.");

        use_effect_with_deps(
            move |_| {
                guard.dispatch(HelmetBridgeGuardAction::Increment);

                move || {
                    guard.dispatch(HelmetBridgeGuardAction::Decrement);
                }
            },
            root,
        );
    }
}

/// Properties of the [HelmetBridge].
#[derive(Properties, PartialEq, Clone)]
pub struct HelmetBridgeProps {
    /// The default title to apply if no title is provided.
    #[prop_or_default]
    pub default_title: Option<AttrValue>,

    /// The function to format title.
    #[prop_or_default]
    pub format_title: Option<FormatTitle>,

    /// The StaticWriter to write to.
    #[cfg(feature = "ssr")]
    #[prop_or_default]
    pub writer: Option<StaticWriter>,
}

impl fmt::Debug for HelmetBridgeProps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelmetBridgeProps")
            .field("default_title", &self.default_title)
            .field(
                "format_title",
                if self.format_title.is_some() {
                    &"Some(_)"
                } else {
                    &"None"
                },
            )
            .finish()
    }
}

/// Renders tags
fn render_tags(
    to_render: BTreeSet<Arc<HelmetTag>>,
    mut last_rendered: Option<BTreeMap<Arc<HelmetTag>, Option<Element>>>,
) -> BTreeMap<Arc<HelmetTag>, Option<Element>> {
    let mut rendered = BTreeMap::new();

    let mut next_last_rendered = None;
    for next_to_render in to_render.into_iter() {
        'inner: loop {
            next_last_rendered = next_last_rendered.or_else(|| {
                last_rendered.as_mut().and_then(|last_rendered| {
                    last_rendered
                        .keys()
                        .next()
                        .cloned()
                        .and_then(|m| last_rendered.remove_entry(&*m))
                })
            });

            match &mut next_last_rendered {
                Some((ref key, ref mut value)) => match (**key).cmp(&next_to_render) {
                    // next_last_rendered key is greater than next_to_render, render next_to_render
                    Ordering::Greater => {
                        let el = next_to_render.apply();

                        rendered.insert(next_to_render, el);

                        break 'inner;
                    }
                    // next_last_rendered key is less than next_to_render, remove next_last_rendered
                    Ordering::Less => {
                        key.detach(value.take());

                        next_last_rendered = None;
                    }
                    // next_last_rendered key is equal to next_to_render, move to
                    // current_rendered
                    Ordering::Equal => {
                        rendered.insert(next_to_render, value.take());

                        next_last_rendered = None;
                        break 'inner;
                    }
                },
                // We have reached the end of all previous render tags, we simply render
                // next_to_render.
                None => {
                    let el = next_to_render.apply();

                    rendered.insert(next_to_render, el);

                    break 'inner;
                }
            }
        }
    }

    if let Some((key, value)) = next_last_rendered {
        key.detach(value);
    }

    if let Some(last_rendered) = last_rendered {
        for (key, value) in last_rendered.into_iter() {
            key.detach(value);
        }
    }

    rendered
}

/// The Helmet Bridge.
///
/// This component is responsible to reconclie all helmet tags to the real dom.
///
/// It accepts two properties, a `default_title` which will be applied when no other title elements
/// are registered and a `format_title` function which is used to format the title before it is
/// passed to the document.
///
/// # Panics
///
/// You can only register 1 `HelmetBridge` per `BounceRoot`. Registering multiple `HelmetBridge`s
/// will panic.
///
/// # Example
///
/// ```
/// # use yew::prelude::*;
/// # use bounce::prelude::*;
/// # use bounce::BounceRoot;
/// # use bounce::helmet::HelmetBridge;
/// #
/// # #[function_component(Comp)]
/// # fn comp() -> Html {
/// html! {
///     <BounceRoot>
///         <HelmetBridge default_title="default title" />
///         // other components.
///     </BounceRoot>
/// }
/// # }
/// ```
#[function_component(HelmetBridge)]
pub fn helmet_bridge(props: &HelmetBridgeProps) -> Html {
    #[cfg(debug_assertions)]
    {
        guard::use_helmet_guard();
    }

    let helmet_states = use_artifacts::<HelmetState>();

    let rendered = use_mut_ref(|| -> Option<BTreeMap<Arc<HelmetTag>, Option<Element>>> { None });

    #[cfg(feature = "ssr")]
    {
        use super::ssr::StaticWriterState;
        use crate::use_atom_setter;

        let writer = props.writer.clone();
        let format_title = props.format_title.clone();
        let default_title = props.default_title.clone();
        let set_static_writer_state = use_atom_setter::<StaticWriterState>();

        use_state(move || {
            set_static_writer_state(StaticWriterState {
                format_title,
                default_title,
                writer,
            })
        });
    }

    // Remove pre-rendered tags.
    use_effect_with_deps(
        |_| {
            let pre_rendered = head()
                .query_selector_all("[data-bounce-helmet=pre-render]")
                .expect_throw("failed to read pre rendered tags");

            for i in 0..pre_rendered.length() {
                if let Some(m) = pre_rendered.get(i) {
                    if let Some(parent) = m.parent_node() {
                        let _ = parent.remove_child(&m);
                    }
                }
            }
        },
        (),
    );

    use_effect_with_deps(
        move |(helmet_states, format_title, default_title)| {
            // Calculate tags to render.
            let to_render =
                merge_helmet_states(helmet_states, format_title.as_ref(), default_title.clone());

            let mut rendered = rendered.borrow_mut();
            *rendered = Some(render_tags(to_render, rendered.take()));

            || {}
        },
        (
            helmet_states,
            props.format_title.clone(),
            props.default_title.clone(),
        ),
    );

    Html::default()
}
