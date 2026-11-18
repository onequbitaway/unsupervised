//! Residents that left the world.

use std::collections::VecDeque;

use unsup_protocol::{ResidentId, ResidentView, ZoneId};

/// The last state of a resident that left the world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Departed {
    /// The last view of the resident.
    pub view: ResidentView,
    /// The last zone of the resident.
    pub zone: ZoneId,
    /// The frame in which it left.
    pub frame: u64,
    /// Why it left, when the kernel said why.
    pub cause: Option<String>,
}

/// A bounded list of the residents that left the world last.
#[derive(Debug)]
pub(crate) struct DepartedList {
    list: VecDeque<Departed>,
    capacity: usize,
}

impl DepartedList {
    /// Makes an empty list.
    pub fn new(capacity: usize) -> DepartedList {
        DepartedList { list: VecDeque::new(), capacity }
    }

    /// Adds a resident. The oldest entry leaves when the list is full.
    pub fn push(&mut self, departed: Departed) {
        if self.capacity == 0 {
            return;
        }
        if self.list.len() >= self.capacity {
            self.list.pop_front();
        }
        self.list.push_back(departed);
    }

    /// Returns the newest entry of a resident.
    pub fn get(&self, id: ResidentId) -> Option<&Departed> {
        self.list.iter().rev().find(|departed| departed.view.id == id)
    }
}

#[cfg(test)]
mod tests {
    use unsup_protocol::Pos;

    use super::*;

    fn departed(id: u64, frame: u64) -> Departed {
        Departed {
            view: ResidentView {
                id: ResidentId(id),
                name: format!("r{id}"),
                pos: Pos::new(0, 0),
                activity: None,
                model: None,
            },
            zone: ZoneId(0),
            frame,
            cause: None,
        }
    }

    #[test]
    fn the_list_keeps_the_newest_entries() {
        let mut list = DepartedList::new(2);
        list.push(departed(1, 10));
        list.push(departed(2, 11));
        list.push(departed(3, 12));
        assert_eq!(list.get(ResidentId(1)), None);
        assert_eq!(list.get(ResidentId(2)).map(|d| d.frame), Some(11));
        assert_eq!(list.get(ResidentId(3)).map(|d| d.frame), Some(12));
    }

    #[test]
    fn a_list_without_room_keeps_nothing() {
        let mut list = DepartedList::new(0);
        list.push(departed(1, 10));
        assert_eq!(list.get(ResidentId(1)), None);
    }
}
