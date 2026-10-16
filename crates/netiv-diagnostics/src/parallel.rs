//! Exécution parallèle de sondes indépendantes.
//!
//! Chaque sonde travaille sur sa propre copie de la simulation : les sondes
//! ne partagent aucun état modifiable et peuvent tourner sur plusieurs cœurs.
//! L'ordre des résultats est celui des entrées, quel que soit le nombre de
//! threads : le résultat reste **déterministe**.

/// En dessous de ce nombre d'éléments par thread, le parallélisme ne paie pas
/// (création des threads, copie de la simulation).
pub const ITEMS_PER_THREAD: usize = 32;

/// Nombre de threads conseillé pour `n` sondes.
pub fn worker_count(n: usize) -> usize {
    let cpus = std::thread::available_parallelism().map_or(1, |c| c.get());
    cpus.min(n / ITEMS_PER_THREAD).max(1)
}

/// Applique `f` à chaque élément, sur `threads` threads (1 = séquentiel).
/// Les résultats sont rendus dans l'ordre de `items`.
pub fn par_map_with<T, R, F>(items: &[T], threads: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let threads = threads.max(1);
    if threads == 1 || items.len() < 2 {
        return items.iter().map(&f).collect();
    }
    let chunk = items.len().div_ceil(threads);
    let f = &f;
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| scope.spawn(move || part.iter().map(f).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    })
}

/// Comme [`par_map_with`], avec le nombre de threads de [`worker_count`].
pub fn par_map<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    par_map_with(items, worker_count(items.len()), f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_is_preserved_whatever_the_thread_count() {
        let items: Vec<u32> = (0..1000).collect();
        let expected: Vec<u32> = items.iter().map(|x| x * 3).collect();
        for threads in [1, 2, 3, 7, 64] {
            assert_eq!(par_map_with(&items, threads, |x| x * 3), expected);
        }
        assert!(par_map_with(&[] as &[u32], 4, |x| *x).is_empty());
    }

    #[test]
    fn small_inputs_stay_sequential() {
        assert_eq!(worker_count(0), 1);
        assert_eq!(worker_count(ITEMS_PER_THREAD - 1), 1);
    }
}
