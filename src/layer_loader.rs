//! Browser-free state machine for per-layer lazy loading.
//!
//! A layer may need data (a texture, a tile set). The first time it is
//! selected, its data is fetched. Until the data is ready the view keeps
//! showing the previous layer; then it switches. A failed fetch falls back
//! to the layer that is still shown, and selecting the layer again retries.
//!
//! The machine owns no I/O. [`LayerLoader::select`] and
//! [`LayerLoader::finished`] return [`Effect`]s that the caller carries out:
//! start a fetch, switch the displayed layer, or report an error. Because
//! the caller reports results by layer, a stale result (a layer that is no
//! longer the wanted one) is cached but never switches the view.

use std::collections::HashMap;
use std::hash::Hash;

/// A layer key that may declare data it needs before it can be shown.
pub trait Loadable: Copy + Eq + Hash {
    /// Whether the layer must fetch data before it can be shown.
    fn needs_data(self) -> bool;
}

/// Load state of one layer's data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataState {
    /// Never requested.
    Idle,
    Loading,
    /// Loaded, or needs no data.
    Ready,
    /// The last fetch failed; the next selection retries.
    Failed,
}

/// What the caller must do after a transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect<K> {
    /// Start fetching this layer's data, then call `finished`.
    Fetch(K),
    /// Make this layer the displayed one.
    Show(K),
    /// Loading this layer failed. The displayed layer is unchanged.
    Failed(K, String),
}

pub struct LayerLoader<K: Loadable> {
    /// The layer on screen.
    shown: K,
    /// The layer the user last asked for. Equals `shown` when idle.
    wanted: K,
    states: HashMap<K, DataState>,
}

impl<K: Loadable> LayerLoader<K> {
    /// Start with `initial` on screen. It must need no data. To start on a
    /// layer that does (the URL names one), begin with a data-free layer and
    /// `select` the requested one.
    pub fn new(initial: K) -> Self {
        LayerLoader {
            shown: initial,
            wanted: initial,
            states: HashMap::new(),
        }
    }

    pub fn shown(&self) -> K {
        self.shown
    }

    /// The layer the user last selected. This is what the URL records.
    pub fn wanted(&self) -> K {
        self.wanted
    }

    pub fn state(&self, layer: K) -> DataState {
        if !layer.needs_data() {
            return DataState::Ready;
        }
        self.states.get(&layer).copied().unwrap_or(DataState::Idle)
    }

    /// The layer being waited for, if the wanted layer is not yet shown.
    pub fn pending(&self) -> Option<K> {
        (self.wanted != self.shown).then_some(self.wanted)
    }

    /// The user picked `layer`. The latest selection always wins.
    pub fn select(&mut self, layer: K) -> Vec<Effect<K>> {
        self.wanted = layer;
        match self.state(layer) {
            DataState::Ready => {
                self.shown = layer;
                vec![Effect::Show(layer)]
            }
            // Already in flight; `finished` will switch if still wanted.
            DataState::Loading => vec![],
            DataState::Idle | DataState::Failed => {
                self.states.insert(layer, DataState::Loading);
                vec![Effect::Fetch(layer)]
            }
        }
    }

    /// The layer's loaded data is gone, for example because another layer
    /// reused its GPU texture. Its next selection fetches again (the caller
    /// may serve that from a cache). A layer that is shown stays shown; a
    /// fetch in flight is not affected.
    pub fn evict(&mut self, layer: K) {
        if self.states.get(&layer) == Some(&DataState::Ready) {
            self.states.insert(layer, DataState::Idle);
        }
    }

    /// A fetch started by [`Effect::Fetch`] completed.
    pub fn finished(&mut self, layer: K, result: Result<(), String>) -> Vec<Effect<K>> {
        if self.states.get(&layer) != Some(&DataState::Loading) {
            return vec![]; // Unknown or duplicate result.
        }
        let current = self.wanted == layer;
        match result {
            Ok(()) => {
                self.states.insert(layer, DataState::Ready);
                if current {
                    self.shown = layer;
                    return vec![Effect::Show(layer)];
                }
                vec![]
            }
            Err(msg) => {
                self.states.insert(layer, DataState::Failed);
                if current {
                    // Fall back to what is on screen.
                    self.wanted = self.shown;
                    return vec![Effect::Failed(layer, msg)];
                }
                vec![]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    enum L {
        Plain,
        Plain2,
        A,
        B,
    }

    impl Loadable for L {
        fn needs_data(self) -> bool {
            matches!(self, L::A | L::B)
        }
    }

    fn loader() -> LayerLoader<L> {
        LayerLoader::new(L::Plain)
    }

    #[test]
    fn data_free_layers_switch_immediately_and_fetch_nothing() {
        let mut l = loader();
        assert_eq!(l.select(L::Plain2), vec![Effect::Show(L::Plain2)]);
        assert_eq!(l.shown(), L::Plain2);
        assert_eq!(l.pending(), None);
    }

    #[test]
    fn first_selection_fetches_and_keeps_previous_layer_until_ready() {
        let mut l = loader();
        assert_eq!(l.select(L::A), vec![Effect::Fetch(L::A)]);
        assert_eq!(l.shown(), L::Plain);
        assert_eq!(l.wanted(), L::A);
        assert_eq!(l.pending(), Some(L::A));
        assert_eq!(l.state(L::A), DataState::Loading);
        assert_eq!(l.finished(L::A, Ok(())), vec![Effect::Show(L::A)]);
        assert_eq!(l.shown(), L::A);
        assert_eq!(l.pending(), None);
    }

    #[test]
    fn unselected_layers_never_fetch() {
        let mut l = loader();
        l.select(L::A);
        l.finished(L::A, Ok(()));
        assert_eq!(l.state(L::B), DataState::Idle);
    }

    #[test]
    fn loaded_layer_is_not_fetched_again() {
        let mut l = loader();
        l.select(L::A);
        l.finished(L::A, Ok(()));
        l.select(L::Plain);
        assert_eq!(l.select(L::A), vec![Effect::Show(L::A)]);
    }

    #[test]
    fn reselecting_while_loading_does_not_fetch_twice() {
        let mut l = loader();
        assert_eq!(l.select(L::A), vec![Effect::Fetch(L::A)]);
        assert_eq!(l.select(L::A), vec![]);
        assert_eq!(l.finished(L::A, Ok(())), vec![Effect::Show(L::A)]);
    }

    #[test]
    fn rapid_switch_final_selection_wins_when_first_finishes_first() {
        let mut l = loader();
        l.select(L::A);
        l.select(L::B);
        // A finishes while B is wanted: cached, but no switch.
        assert_eq!(l.finished(L::A, Ok(())), vec![]);
        assert_eq!(l.shown(), L::Plain);
        assert_eq!(l.finished(L::B, Ok(())), vec![Effect::Show(L::B)]);
        assert_eq!(l.shown(), L::B);
    }

    #[test]
    fn rapid_switch_final_selection_wins_when_first_finishes_last() {
        let mut l = loader();
        l.select(L::A);
        l.select(L::B);
        assert_eq!(l.finished(L::B, Ok(())), vec![Effect::Show(L::B)]);
        // The stale A result must not switch the view back.
        assert_eq!(l.finished(L::A, Ok(())), vec![]);
        assert_eq!(l.shown(), L::B);
        assert_eq!(l.wanted(), L::B);
        // A is cached for later.
        assert_eq!(l.select(L::A), vec![Effect::Show(L::A)]);
    }

    #[test]
    fn selecting_a_ready_layer_cancels_a_pending_switch() {
        let mut l = loader();
        l.select(L::A);
        assert_eq!(l.select(L::Plain), vec![Effect::Show(L::Plain)]);
        assert_eq!(l.pending(), None);
        assert_eq!(l.finished(L::A, Ok(())), vec![]);
        assert_eq!(l.shown(), L::Plain);
    }

    #[test]
    fn failure_falls_back_and_next_selection_retries() {
        let mut l = loader();
        l.select(L::A);
        assert_eq!(
            l.finished(L::A, Err("404".into())),
            vec![Effect::Failed(L::A, "404".into())]
        );
        assert_eq!(l.shown(), L::Plain);
        assert_eq!(l.wanted(), L::Plain);
        assert_eq!(l.state(L::A), DataState::Failed);
        assert_eq!(l.select(L::A), vec![Effect::Fetch(L::A)]);
        assert_eq!(l.finished(L::A, Ok(())), vec![Effect::Show(L::A)]);
    }

    #[test]
    fn stale_failure_is_silent_but_allows_retry() {
        let mut l = loader();
        l.select(L::A);
        l.select(L::Plain2);
        assert_eq!(l.finished(L::A, Err("boom".into())), vec![]);
        assert_eq!(l.shown(), L::Plain2);
        assert_eq!(l.select(L::A), vec![Effect::Fetch(L::A)]);
    }

    #[test]
    fn failure_falls_back_to_the_previously_loaded_layer() {
        let mut l = loader();
        l.select(L::A);
        l.finished(L::A, Ok(()));
        l.select(L::B);
        l.finished(L::B, Err("x".into()));
        assert_eq!(l.shown(), L::A);
        assert_eq!(l.wanted(), L::A);
    }

    #[test]
    fn unexpected_or_duplicate_results_are_ignored() {
        let mut l = loader();
        assert_eq!(l.finished(L::A, Ok(())), vec![]);
        l.select(L::A);
        l.finished(L::A, Ok(()));
        assert_eq!(l.finished(L::A, Err("late".into())), vec![]);
        assert_eq!(l.state(L::A), DataState::Ready);
    }

    #[test]
    fn evicted_layer_fetches_again_and_stays_shown_meanwhile() {
        let mut l = loader();
        l.select(L::A);
        l.finished(L::A, Ok(()));
        l.select(L::B);
        assert_eq!(l.finished(L::B, Ok(())), vec![Effect::Show(L::B)]);
        // B's data replaced A's (one shared texture).
        l.evict(L::A);
        assert_eq!(l.state(L::A), DataState::Idle);
        assert_eq!(l.shown(), L::B);
        assert_eq!(l.select(L::A), vec![Effect::Fetch(L::A)]);
        assert_eq!(l.shown(), L::B, "B stays on screen until A is back");
        assert_eq!(l.finished(L::A, Ok(())), vec![Effect::Show(L::A)]);
    }

    #[test]
    fn evict_leaves_loading_failed_and_data_free_layers_alone() {
        let mut l = loader();
        l.select(L::A);
        l.evict(L::A);
        assert_eq!(l.state(L::A), DataState::Loading);
        assert_eq!(l.finished(L::A, Ok(())), vec![Effect::Show(L::A)]);
        l.select(L::B);
        l.finished(L::B, Err("x".into()));
        l.evict(L::B);
        assert_eq!(l.state(L::B), DataState::Failed);
        l.evict(L::Plain);
        assert_eq!(l.state(L::Plain), DataState::Ready);
        assert_eq!(l.select(L::Plain), vec![Effect::Show(L::Plain)]);
    }

    #[test]
    fn stale_result_can_be_evicted_so_it_is_not_shown_from_missing_data() {
        // A finishes after B was chosen; the caller did not keep A's data on
        // the GPU, so it evicts A and the next selection loads it again.
        let mut l = loader();
        l.select(L::A);
        l.select(L::B);
        l.finished(L::B, Ok(()));
        assert_eq!(l.finished(L::A, Ok(())), vec![]);
        l.evict(L::A);
        assert_eq!(l.select(L::A), vec![Effect::Fetch(L::A)]);
    }

    #[test]
    fn startup_layer_goes_through_select() {
        // `&layer=a` at startup: shown stays on the default until ready.
        let mut l = loader();
        assert_eq!(l.select(L::A), vec![Effect::Fetch(L::A)]);
        assert_eq!(l.shown(), L::Plain);
    }
}
