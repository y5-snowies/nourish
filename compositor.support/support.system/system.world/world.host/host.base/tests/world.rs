//! End-to-end kernel behavior: read-only lifecycle storage with buffer-only
//! mutation, owner-announced events with listener fan-out, input priority
//! order, and the enable/disable lifecycle.

use compositor_support_system_buffer_token_base::y5_buffer;
use compositor_support_system_channel_token_base::y5_channel;
use compositor_support_system_input_event_base::base::{InputEvent, InputFlow, Modality};
use compositor_support_system_input_layer_base::base as input_layer;
use compositor_support_system_storage_token_base::y5_storage;
use compositor_support_system_trait_system_base::base::{BufferCx, System, SystemCx, WorldBuilder};
use compositor_support_system_world_frame_base::base::{FramePlan, FrameTick, Layer};
use compositor_support_system_storage_slot_base::base::Storage;
use compositor_support_system_world_host_base::base::World;
use std::any::Any;
use std::time::Duration;

#[derive(Default)]
pub struct CounterData {
    pub value: u32,
    pub log: Vec<&'static str>,
}

y5_storage!(pub COUNTER, COUNTER_MUT: CounterData);
y5_channel!(pub BUMP, BUMP_TX: u32);
y5_channel!(pub BUMPED, BUMPED_TX: u32);

enum CounterCmd {
    Log(&'static str),
    Add(u32),
}
y5_buffer!(COUNTER_BUF: CounterCmd);

/// Owns COUNTER. Lifecycle methods only READ storage and write buffer
/// messages; `buffer()` is the single mutation site. Announces BUMPED;
/// also listens to its own BUMPED to prove fan-out.
struct CounterSystem;

impl System for CounterSystem {
    fn name(&self) -> &'static str {
        "counter"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        builder.storage.insert(&COUNTER, CounterData::default());
        builder.receive(&BUMP, Self::on_bump);
        builder.receive(&BUMPED, Self::on_bumped_self);
        builder.input(input_layer::WORLD);
    }

    fn start(&mut self, cx: &mut SystemCx) {
        cx.write(&COUNTER_BUF, CounterCmd::Log("start"));
    }

    fn on_enable(&mut self, cx: &mut SystemCx) {
        cx.write(&COUNTER_BUF, CounterCmd::Log("enable"));
    }

    fn on_disable(&mut self, cx: &mut SystemCx) {
        cx.write(&COUNTER_BUF, CounterCmd::Log("disable"));
    }

    fn input(&mut self, cx: &mut SystemCx, _event: &InputEvent) -> InputFlow {
        cx.write(&COUNTER_BUF, CounterCmd::Log("input:world"));
        InputFlow::Consume
    }

    fn draw(&mut self, _cx: &mut SystemCx, plan: &mut FramePlan) {
        plan.push(Layer(400), Box::new("counter-node"));
    }

    fn buffer(&mut self, cx: &mut BufferCx, message: Box<dyn Any>) {
        let data = cx.storage.get_mut(&COUNTER_MUT);
        match *message.downcast::<CounterCmd>().expect("counter buffer type") {
            CounterCmd::Log(entry) => data.log.push(entry),
            CounterCmd::Add(amount) => data.value += amount,
        }
    }
}

impl CounterSystem {
    fn on_bump(&mut self, cx: &mut SystemCx, amount: &u32) {
        // Read-only storage: announce from the read + queue the mutation.
        // Flush runs before the next delivery, so the second BUMP reads the
        // first one's applied value.
        let value = cx.storage.get(&COUNTER).value + amount;
        cx.send(&BUMPED_TX, value);
        cx.write(&COUNTER_BUF, CounterCmd::Add(*amount));
    }

    fn on_bumped_self(&mut self, cx: &mut SystemCx, _value: &u32) {
        cx.write(&COUNTER_BUF, CounterCmd::Log("bumped"));
    }
}

/// Registered LAST but on the OVERLAY layer — sees input FIRST. Reads the
/// counter through the public token (cross-system read) and passes.
struct OverlaySystem;

y5_storage!(pub SEEN, SEEN_MUT: Vec<u32>);
y5_buffer!(SEEN_BUF: u32);

impl System for OverlaySystem {
    fn name(&self) -> &'static str {
        "overlay"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        builder.storage.insert(&SEEN, Vec::new());
        builder.receive(&BUMPED, Self::on_bumped);
        builder.input(input_layer::OVERLAY);
    }

    fn input(&mut self, cx: &mut SystemCx, _event: &InputEvent) -> InputFlow {
        let counter = cx.storage.get(&COUNTER).value; // other module's token
        assert_eq!(counter, 3, "overlay reads counter state via public token");
        InputFlow::Pass
    }

    fn buffer(&mut self, cx: &mut BufferCx, message: Box<dyn Any>) {
        let value = *message.downcast::<u32>().expect("seen buffer type");
        cx.storage.get_mut(&SEEN_MUT).push(value);
    }
}

impl OverlaySystem {
    fn on_bumped(&mut self, cx: &mut SystemCx, value: &u32) {
        cx.write(&SEEN_BUF, *value);
    }
}

#[test]
fn world_end_to_end() {
    let kernel = Storage::new();
    let mut world = World::build(uuid::Uuid::now_v7(), "test", vec![Box::new(CounterSystem), Box::new(OverlaySystem)], &kernel);

    // start() ran once after all registers; its buffer write applied
    assert_eq!(world.storage().get(&COUNTER).log, vec!["start"]);

    // events + cascade + buffer ordering: BUMP(1), BUMP(2) -> value 3;
    // both listeners observed each BUMPED (fan-out)
    world.channels().send(&BUMP_TX, 1);
    world.channels().send(&BUMP_TX, 2);
    world.dispatch(&kernel);
    assert_eq!(world.storage().get(&COUNTER).value, 3);
    assert_eq!(world.storage().get(&SEEN), &vec![1, 3]);
    assert_eq!(world.storage().get(&COUNTER).log, vec!["start", "bumped", "bumped"]);

    // input: overlay (registered last, higher layer) runs first and passes;
    // counter consumes at WORLD layer; its buffered log applied
    let event = InputEvent::PointerButton {
        button: 0x110,
        pressed: true,
        x: 1.0,
        y: 2.0,
        modality: Modality::Mouse,
    };
    assert_eq!(world.input(&kernel, &event, None, None), InputFlow::Consume);
    assert_eq!(
        world.storage().get(&COUNTER).log,
        vec!["start", "bumped", "bumped", "input:world"]
    );

    // update + draw
    world.update(&kernel, &FrameTick { index: 1, delta: Duration::from_millis(16) }, None, None);
    let mut plan = FramePlan::new();
    world.draw(&kernel, &mut plan, None);
    let nodes = plan.sorted();
    assert_eq!(nodes.len(), 1);
    assert_eq!(*nodes[0].1.downcast_ref::<&str>().unwrap(), "counter-node");

    // lifecycle
    world.disable(&kernel);
    world.enable(&kernel);
    let log = &world.storage().get(&COUNTER).log;
    assert!(log.ends_with(&["disable", "enable"]));
}
