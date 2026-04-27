// Closed-set dispatch via enum + match, contrasted with `Box<dyn Trait>`.
// When the variant set is fixed at the library boundary, an enum compiles
// to direct branches (or a jump table for larger sets) — no vtable, no
// indirect call, exhaustive matches surface missed variants at compile
// time, and `Vec<Command>` packs each element into one variant slot
// rather than indirecting through a fat pointer.
//
// Use `Box<dyn Trait>` only when external code must add new variants;
// for closed sets that the library owns, the enum representation is
// strictly cheaper to dispatch and friendlier to inline.

#[derive(Debug, Clone, Copy)]
pub enum Command {
    Increment(u32),
    Reset,
    Set(u32),
}

#[derive(Debug, Default)]
pub struct Counter {
    value: u32,
}

impl Counter {
    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Increment(by) => self.value = self.value.saturating_add(by),
            Command::Reset => self.value = 0,
            Command::Set(v) => self.value = v,
        }
    }

    pub fn run(&mut self, batch: &[Command]) {
        for cmd in batch {
            self.apply(*cmd);
        }
    }

    pub fn value(&self) -> u32 {
        self.value
    }
}
