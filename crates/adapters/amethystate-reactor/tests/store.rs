use amethystate::test_utils::unique_store;
use amethystate::{ReactiveMap, amethystate};
use amethystate_reactor::AmeCx;
use std::cell::Cell;
use std::rc::Rc;
use windows_reactor::{ChannelDispatcher, RenderCx, UiRerenderGuard};

#[amethystate(prefix = "reactor_test")]
pub struct Settings {
    #[amestate(default = 8080)]
    pub port: u16,

    pub widths: ReactiveMap<String, u64>,
}

struct Host {
    cx: RenderCx,
    dispatcher: ChannelDispatcher,
    _rerenders: Rc<Cell<u32>>,
    _guard: UiRerenderGuard,
}

fn host() -> Host {
    let dispatcher = ChannelDispatcher::new();
    let mut cx = RenderCx::for_test();
    cx.set_marshaller(Some(dispatcher.marshaller()));

    let rerenders = Rc::new(Cell::new(0));
    let counted = Rc::clone(&rerenders);
    let guard = UiRerenderGuard::install(
        cx.host_id(),
        Rc::new(move || counted.set(counted.get() + 1)),
    );

    Host {
        cx,
        dispatcher,
        _rerenders: rerenders,
        _guard: guard,
    }
}

#[test]
fn a_field_change_reaches_the_next_render() {
    let store = unique_store("reactor_field");
    let mut host = host();

    host.cx.begin_render();
    let state: Settings = host.cx.use_ame_state_in(&store);
    assert_eq!(host.cx.use_ame(&state.port()), 8080);
    host.cx.flush_effects();

    state.port().set(9090).unwrap();
    host.dispatcher.drain();

    host.cx.begin_render();
    let state: Settings = host.cx.use_ame_state_in(&store);
    assert_eq!(host.cx.use_ame(&state.port()), 9090);
}

#[test]
fn the_slice_loads_once_across_renders() {
    let store = unique_store("reactor_once");
    let mut host = host();

    host.cx.begin_render();
    let first: Settings = host.cx.use_ame_state_in(&store);

    host.cx.begin_render();
    let again: Settings = host.cx.use_ame_state_in(&store);

    assert!(
        first.port() == again.port(),
        "the memoised handle, not a fresh load"
    );
}

#[test]
fn a_map_renders_sorted_entries() {
    let store = unique_store("reactor_map");
    let mut host = host();

    host.cx.begin_render();
    let state: Settings = host.cx.use_ame_state_in(&store);
    let entries = host.cx.use_ame(&state.widths());
    assert!(entries.is_empty());
    host.cx.flush_effects();

    state.widths().insert("zulu".into(), &3).unwrap();
    state.widths().insert("alpha".into(), &1).unwrap();
    host.dispatcher.drain();

    host.cx.begin_render();
    let state: Settings = host.cx.use_ame_state_in(&store);
    let entries = host.cx.use_ame(&state.widths());

    assert_eq!(
        entries,
        vec![("alpha".to_string(), 1), ("zulu".to_string(), 3)]
    );
}

#[test]
fn an_entry_follows_one_key() {
    let store = unique_store("reactor_entry");
    let mut host = host();

    host.cx.begin_render();
    let state: Settings = host.cx.use_ame_state_in(&store);
    assert_eq!(
        host.cx.use_ame_entry(&state.widths(), "cpu".to_string()),
        None
    );
    host.cx.flush_effects();

    state.widths().insert("mem".into(), &99).unwrap();
    state.widths().insert("cpu".into(), &110).unwrap();
    host.dispatcher.drain();

    host.cx.begin_render();
    let state: Settings = host.cx.use_ame_state_in(&store);
    assert_eq!(
        host.cx.use_ame_entry(&state.widths(), "cpu".to_string()),
        Some(110),
        "the other key does not leak in"
    );
}
