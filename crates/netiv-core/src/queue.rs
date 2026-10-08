//! File d'événements ordonnée par `(temps, numéro d'insertion)`.
//!
//! À temps égal, l'ordre est celui de l'insertion : c'est ce qui rend la
//! simulation déterministe.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use crate::time::SimTime;

/// Un élément de la file avec sa date et son rang d'insertion.
#[derive(Debug, Clone)]
pub struct Scheduled<E> {
    /// Date d'exécution.
    pub time: SimTime,
    /// Rang d'insertion, unique et croissant.
    pub seq: u64,
    /// Contenu.
    pub event: E,
}

// L'ordre ignore `event` : seuls `(time, seq)` comptent, et `seq` est unique.
impl<E> PartialEq for Scheduled<E> {
    fn eq(&self, other: &Self) -> bool {
        self.seq == other.seq
    }
}
impl<E> Eq for Scheduled<E> {}
impl<E> PartialOrd for Scheduled<E> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<E> Ord for Scheduled<E> {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.time, self.seq).cmp(&(other.time, other.seq))
    }
}

/// File de priorité d'événements, du plus ancien au plus récent.
#[derive(Debug, Clone)]
pub struct EventQueue<E> {
    heap: BinaryHeap<Reverse<Scheduled<E>>>,
    next_seq: u64,
}

impl<E> Default for EventQueue<E> {
    fn default() -> Self {
        EventQueue {
            heap: BinaryHeap::new(),
            next_seq: 0,
        }
    }
}

impl<E> EventQueue<E> {
    /// File vide.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insère un événement à la date `time`. Retourne son rang d'insertion.
    pub fn push(&mut self, time: SimTime, event: E) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.heap.push(Reverse(Scheduled { time, seq, event }));
        seq
    }

    /// Retire et retourne le prochain événement.
    pub fn pop(&mut self) -> Option<Scheduled<E>> {
        self.heap.pop().map(|Reverse(s)| s)
    }

    /// Date du prochain événement, sans le retirer.
    pub fn peek_time(&self) -> Option<SimTime> {
        self.heap.peek().map(|Reverse(s)| s.time)
    }

    /// Nombre d'événements en attente.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// `true` si aucun événement n'est en attente.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(q: &mut EventQueue<&'static str>) -> Vec<&'static str> {
        std::iter::from_fn(|| q.pop().map(|s| s.event)).collect()
    }

    #[test]
    fn pops_in_time_order() {
        let mut q = EventQueue::new();
        q.push(SimTime(30), "c");
        q.push(SimTime(10), "a");
        q.push(SimTime(20), "b");
        assert_eq!(drain(&mut q), ["a", "b", "c"]);
    }

    #[test]
    fn simultaneous_events_keep_insertion_order() {
        let mut q = EventQueue::new();
        q.push(SimTime(5), "first");
        q.push(SimTime(5), "second");
        q.push(SimTime(1), "earlier");
        q.push(SimTime(5), "third");
        assert_eq!(drain(&mut q), ["earlier", "first", "second", "third"]);
    }

    #[test]
    fn seq_is_strictly_increasing() {
        let mut q = EventQueue::new();
        assert_eq!(q.push(SimTime(9), ()), 0);
        assert_eq!(q.push(SimTime(1), ()), 1);
    }

    #[test]
    fn peek_len_and_empty() {
        let mut q = EventQueue::new();
        assert!(q.is_empty());
        assert_eq!(q.peek_time(), None);
        q.push(SimTime(7), ());
        q.push(SimTime(3), ());
        assert_eq!(q.len(), 2);
        assert_eq!(q.peek_time(), Some(SimTime(3)));
    }
}
