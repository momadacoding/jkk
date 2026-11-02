//! 「远程默认分支」的一次性预取。
//!
//! 删远程分支前的护栏（[`crate::engine::branch::ensure_deletable_on_remote`]）要问一次
//! 远程 `ls-remote --symref`，这一趟是一次完整的 SSH 握手——实测到 GitHub 3 秒多，
//! 和紧接着的 `push --delete` 一样贵，删一次远程就成了两倍耗时。
//!
//! 办法是在用户**表明要删远程时**（确认框打开、或勾上「同时删除远程」）就在后台发
//! 这次查询，点确认时护栏取走结果；查询还在路上就限时等它，不另发一次。
//!
//! 安全边界（护栏不能因此失效）：
//! - **结果一次性。** 护栏取走即消失，它自己查到的也不存。一份答案只服务一次删除，
//!   不会变成 [`TTL`] 内所有删除共用的缓存：管理员在两次删除之间改了默认分支，第二次
//!   要么有自己的新预取，要么当场问。
//! - **只存成功结果，失败不留痕。** 护栏等到的如果是失败，会自己再问一次。不能沿用：
//!   预取可能是断网时发的，用户重连后点确认，此刻 push 已经能到达远程，而本地又可能
//!   没有 `refs/remotes/<remote>/HEAD` 可兜底——沿用失败等于放行默认分支。
//! - **结果超过 [`TTL`] 就作废**，按查询发出时刻计，没有例外。默认分支被改的感知窗口从
//!   「ls-remote 到 push 之间的几秒」放宽到「最多 TTL」；超过就当场重查，只是慢，不会松。
//! - **等待有上限**（[`MAX_WAIT`]），且查询期间不持锁。`run_git` 没有超时，半开的 TCP
//!   连接或没人理的凭据窗口能让一次查询永远不返回，卡住的预取不能把之后的删除一起拖死。
//!   等不到就自己问：结论和没有预取时一样，只是多等了一会儿。
//! - key 含 push 目标的地址：改了 URL 或 pushurl 之后，旧结果不会拿去给新目标把关。
//!
//! 删除序列的第一步不可撤销时，会先用 [`PrefetchCache::peek_or_query`] 预检一次。它只是
//! 把取走的时机推迟到真正的删除，上面几条不变：答案仍然只被一次删除取走，TTL 仍按查询
//! 发出时刻算，所以预检和删除之间隔得太久（worktree 很大）时护栏会当场重问。

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// 预取结果的有效期。
///
/// 覆盖「表明要删 → 读完确认框 → 点确认」的正常节奏就够了，没必要更长：长了只是把
/// 「远程刚改了默认分支、这边还拿旧名字把关」的窗口拉大，换不来别的好处。
pub const TTL: Duration = Duration::from_secs(30);

/// 护栏最多等一次预取到它发出后多久，超过就自己问。
///
/// 正常一次握手 3~4 秒，留足慢链路的余量。必须小于 [`TTL`]：等到的结果还得在有效期内。
/// 代价是预取卡死时，这次删除比没有预取时最多多等这么久。
pub const MAX_WAIT: Duration = Duration::from_secs(15);

const _: () = assert!(MAX_WAIT.as_secs() < TTL.as_secs());

/// 一次成功的预取。
#[derive(Debug)]
struct Prefetched {
    /// 查询发出的时刻。过期按它算：远程状态是在发出之后某一刻读到的，偏保守。
    started: Instant,
    value: String,
}

impl Prefetched {
    /// 护栏不能松的那条规则，所有读结果的路径都走这里。
    fn is_within(&self, ttl: Duration) -> bool {
        self.started.elapsed() < ttl
    }
}

#[derive(Debug, Default)]
struct State {
    /// 正在路上的预取的发出时刻。预取绝不替换它，所以同一个 key 永远最多一个。
    in_flight: Option<Instant>,
    last: Option<Prefetched>,
    /// 正阻塞在 `changed` 上的线程数。只给测试用来确定「护栏真的在等了」。
    #[cfg(test)]
    waiters: usize,
}

impl State {
    /// 记下一次成功的查询。预取和预检可能同时在路上，后回来的不一定是后发出的，
    /// 按发出时刻取新，不能拿旧答案盖掉新答案。
    fn remember(&mut self, started: Instant, value: String) {
        if self.last.as_ref().map_or(true, |p| p.started < started) {
            self.last = Some(Prefetched { started, value });
        }
    }
}

#[derive(Debug, Default)]
struct Slot {
    state: Mutex<State>,
    changed: Condvar,
}

impl Slot {
    /// 中毒只可能来自另一个线程在持锁时 panic；这里持锁只做几行字段读写，数据是完整的，
    /// 继续用即可，不能让一次 panic 把之后所有删远程分支都卡死。
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 等路上的预取结束，最多等到它发出后 `max_wait`。
    fn wait_for_prefetch(&self, max_wait: Duration) -> MutexGuard<'_, State> {
        let mut state = self.lock();
        if let Some(started) = state.in_flight {
            // 截止时刻只算一次：等待期间又来一次新预取时，不能重新开始计时
            let deadline = started + max_wait;
            while state.in_flight.is_some() {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                #[cfg(test)]
                {
                    state.waiters += 1;
                }
                let (next, _) = self
                    .changed
                    .wait_timeout(state, left)
                    .unwrap_or_else(|e| e.into_inner());
                state = next;
                #[cfg(test)]
                {
                    state.waiters -= 1;
                }
            }
        }
        state
    }
}

/// 按 key 存放预取结果。
#[derive(Default)]
pub struct PrefetchCache {
    slots: Mutex<HashMap<String, Arc<Slot>>>,
}

impl PrefetchCache {
    fn slot(&self, key: &str) -> Arc<Slot> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.entry(key.to_string()).or_default().clone()
    }

    /// 护栏用：先等路上的预取（最多等到它发出后 `max_wait`），再取走未过期的结果；
    /// 取不到（没预取、过期、预取失败、等超时）就自己查，查到的不存。
    ///
    /// 先等再看结果：路上那次是为这次删除发的刷新，比手里的旧结果新。
    pub fn take_or_query(
        &self,
        key: &str,
        ttl: Duration,
        max_wait: Duration,
        query: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        let slot = self.slot(key);
        let mut state = slot.wait_for_prefetch(max_wait);
        let taken = state.last.take().filter(|p| p.is_within(ttl));
        drop(state);
        match taken {
            Some(p) => Some(p.value),
            None => query(),
        }
    }

    /// 预检用：等待和判断同 [`take_or_query`](Self::take_or_query)，但答案留下来给紧接着的
    /// 删除，自己查到的成功结果也存（按发出时刻计 TTL）。查询期间不持锁。
    pub fn peek_or_query(
        &self,
        key: &str,
        ttl: Duration,
        max_wait: Duration,
        query: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        let slot = self.slot(key);
        let state = slot.wait_for_prefetch(max_wait);
        if let Some(p) = state.last.as_ref().filter(|p| p.is_within(ttl)) {
            return Some(p.value.clone());
        }
        drop(state);

        let started = Instant::now();
        let value = query()?;
        slot.lock().remember(started, value.clone());
        Some(value)
    }

    /// 预取用：有查询在路上、或结果还很新时直接返回，否则查一次，成功才记下。
    ///
    /// 绝不等待，也绝不替换在路上的查询，哪怕它已经卡死：否则用户反复开关确认框，卡死的
    /// git/ssh 进程会一直累积。这个 key 上的预取因此失效时，护栏等够上限后自己问，
    /// 退化成没有预取的样子。
    ///
    /// 「很新」只算 `ttl` 的一半：快过期的结果留着不刷新，用户点确认时已经过期，
    /// 护栏又要全价握手一次。
    pub fn prefetch(&self, key: &str, ttl: Duration, query: impl FnOnce() -> Option<String>) {
        let slot = self.slot(key);
        let mut state = slot.lock();
        let fresh = state.last.as_ref().is_some_and(|p| p.is_within(ttl / 2));
        if fresh || state.in_flight.is_some() {
            return;
        }
        let started = Instant::now();
        state.in_flight = Some(started);
        drop(state);

        // 不持锁地跑：卡住的查询不能挡住护栏看状态、判超时
        let value = query();

        let mut state = slot.lock();
        state.in_flight = None;
        if let Some(value) = value {
            state.remember(started, value);
        }
        drop(state);
        slot.changed.notify_all();
    }
}

/// 全进程共享的那一份。护栏和预取命令必须用同一个实例，否则预取白做。
pub fn global() -> &'static PrefetchCache {
    static CACHE: OnceLock<PrefetchCache> = OnceLock::new();
    CACHE.get_or_init(PrefetchCache::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Sender};
    use std::thread;

    const LONG: Duration = Duration::from_secs(60);

    /// 在后台跑一次停在半路的预取，返回时它已经发出去了。
    ///
    /// 用信号而不是 sleep 排先后：满载跑 1500 个测试时线程调度能超过任何写死的毫秒数。
    fn start_prefetch(
        cache: &Arc<PrefetchCache>,
        value: Option<&'static str>,
    ) -> (thread::JoinHandle<()>, Sender<()>) {
        let (entered_tx, entered_rx) = channel();
        let (release_tx, release_rx) = channel::<()>();
        let c = cache.clone();
        let handle = thread::spawn(move || {
            c.prefetch("k", TTL, move || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                value.map(String::from)
            });
        });
        entered_rx.recv().unwrap();
        (handle, release_tx)
    }

    /// 自旋到有人阻塞在等待上：确定「后到的调用者真的在等」，而不是还没走到那一步。
    fn wait_for_waiter(cache: &PrefetchCache) {
        let slot = cache.slot("k");
        while slot.lock().waiters == 0 {
            thread::yield_now();
        }
    }

    fn with_aged_result(cache: &PrefetchCache, age_secs: u64) {
        cache.slot("k").lock().last = Some(Prefetched {
            started: Instant::now() - Duration::from_secs(age_secs),
            value: "old".into(),
        });
    }

    #[test]
    fn the_guard_takes_a_prefetched_result_without_querying() {
        let cache = PrefetchCache::default();
        cache.prefetch("k", LONG, || Some("main".into()));
        let got = cache.take_or_query("k", LONG, LONG, || panic!("预取过了，不该再问"));
        assert_eq!(got.as_deref(), Some("main"));
    }

    /// 一份答案只服务一次删除：两次删除之间默认分支可能被改了。
    #[test]
    fn a_prefetched_result_serves_only_one_delete() {
        let cache = PrefetchCache::default();
        cache.prefetch("k", LONG, || Some("master".into()));
        cache.take_or_query("k", LONG, LONG, || panic!("预取过了，不该再问"));

        let second = cache.take_or_query("k", LONG, LONG, || Some("main".into()));
        assert_eq!(second.as_deref(), Some("main"));
    }

    /// 护栏自己问到的答案不留给下一次删除。
    #[test]
    fn the_guards_own_answer_is_not_kept() {
        let cache = PrefetchCache::default();
        cache.take_or_query("k", LONG, LONG, || Some("master".into()));
        let second = cache.take_or_query("k", LONG, LONG, || Some("main".into()));
        assert_eq!(second.as_deref(), Some("main"));
    }

    #[test]
    fn a_failed_prefetch_leaves_nothing_behind() {
        let cache = PrefetchCache::default();
        cache.prefetch("k", LONG, || None);
        let got = cache.take_or_query("k", LONG, LONG, || Some("main".into()));
        assert_eq!(got.as_deref(), Some("main"));
    }

    #[test]
    fn an_expired_result_is_not_used() {
        let cache = PrefetchCache::default();
        with_aged_result(&cache, 40);
        let got = cache.take_or_query("k", TTL, LONG, || Some("main".into()));
        assert_eq!(got.as_deref(), Some("main"));
    }

    #[test]
    fn different_keys_do_not_share_results() {
        let cache = PrefetchCache::default();
        cache.prefetch("origin@a", LONG, || Some("master".into()));
        let got = cache.take_or_query("origin@b", LONG, LONG, || Some("trunk".into()));
        assert_eq!(got.as_deref(), Some("trunk"));
    }

    /// 预取还在路上时点了确认：护栏要等那次查询，而不是另发一次（那样又是一趟握手）。
    #[test]
    fn a_waiter_uses_the_result_of_the_prefetch_in_flight() {
        let cache = Arc::new(PrefetchCache::default());
        let (prefetch, release) = start_prefetch(&cache, Some("main"));

        let c = cache.clone();
        let guard = thread::spawn(move || {
            c.take_or_query("k", LONG, LONG, || panic!("等待者不该再问一次"))
        });
        wait_for_waiter(&cache);
        release.send(()).unwrap();

        prefetch.join().unwrap();
        assert_eq!(guard.join().unwrap().as_deref(), Some("main"));
    }

    /// 等到的是失败：护栏必须自己再问一次，不能沿用。
    ///
    /// 这次失败可能发生在用户重连之前。此刻 push 已经能到达远程，沿用失败等于放行。
    #[test]
    fn a_waiter_does_not_trust_the_failure_of_the_prefetch_it_waited_for() {
        let cache = Arc::new(PrefetchCache::default());
        let (prefetch, release) = start_prefetch(&cache, None);

        let c = cache.clone();
        let guard = thread::spawn(move || c.take_or_query("k", LONG, LONG, || Some("main".into())));
        wait_for_waiter(&cache);
        release.send(()).unwrap();

        prefetch.join().unwrap();
        assert_eq!(guard.join().unwrap().as_deref(), Some("main"));
    }

    /// 路上那次是为这次删除发的刷新：护栏要等它，不能拿手里的旧结果就走。
    #[test]
    fn a_waiter_prefers_the_refresh_in_flight_over_an_aged_result() {
        let cache = Arc::new(PrefetchCache::default());
        with_aged_result(&cache, 20);
        let (prefetch, release) = start_prefetch(&cache, Some("new"));

        let c = cache.clone();
        let guard = thread::spawn(move || c.take_or_query("k", TTL, LONG, || panic!("不该再问")));
        wait_for_waiter(&cache);
        release.send(()).unwrap();

        prefetch.join().unwrap();
        assert_eq!(guard.join().unwrap().as_deref(), Some("new"));
    }

    /// 卡死的预取（`run_git` 没有超时）不能把护栏拖死：等够上限就自己问。
    #[test]
    fn a_stuck_prefetch_cannot_block_the_guard() {
        let cache = Arc::new(PrefetchCache::default());
        let (prefetch, release) = start_prefetch(&cache, None);

        // 放行在护栏返回之后才发：只有限时等待真的超时，这里才走得下去
        let got = cache.take_or_query("k", LONG, Duration::from_millis(50), || Some("main".into()));
        assert_eq!(got.as_deref(), Some("main"));

        release.send(()).unwrap();
        prefetch.join().unwrap();
    }

    /// 已有查询在路上时预取直接返回，不占着线程排队。
    ///
    /// 放行在预取返回之后才发：预取一旦等待，这个测试会挂住而不是偶发通过。
    #[test]
    fn a_prefetch_never_waits_behind_a_query_in_flight() {
        let cache = Arc::new(PrefetchCache::default());
        let (prefetch, release) = start_prefetch(&cache, Some("main"));

        cache.prefetch("k", LONG, || panic!("已有查询在路上，不该再发一次"));

        release.send(()).unwrap();
        prefetch.join().unwrap();
    }

    /// 路上那次早已超时（多半卡死了）也不替换：否则反复开关确认框会累积卡死的进程。
    #[test]
    fn a_prefetch_does_not_pile_up_behind_a_stuck_query() {
        let cache = PrefetchCache::default();
        cache.slot("k").lock().in_flight = Some(Instant::now() - Duration::from_secs(20));

        cache.prefetch("k", LONG, || panic!("不能再叠一个卡死的查询"));
    }

    /// 快过期的结果要刷新：留到用户点确认时已经过期，护栏又要全价握手一次。
    #[test]
    fn a_prefetch_refreshes_a_result_past_half_of_its_ttl() {
        let cache = PrefetchCache::default();
        with_aged_result(&cache, 20);

        cache.prefetch("k", TTL, || Some("new".into()));

        let got = cache.take_or_query("k", TTL, LONG, || panic!("预取已经刷新了"));
        assert_eq!(got.as_deref(), Some("new"));
    }

    #[test]
    fn a_prefetch_skips_when_the_result_is_still_fresh() {
        let cache = PrefetchCache::default();
        cache.prefetch("k", LONG, || Some("main".into()));
        cache.prefetch("k", LONG, || panic!("结果还新鲜，不该再发一次"));
    }

    /// 预检看过的预取结果还在，紧接着的删除能取走，而且只有这一次删除能取走。
    #[test]
    fn a_peek_leaves_the_result_for_exactly_one_delete() {
        let cache = PrefetchCache::default();
        cache.prefetch("k", LONG, || Some("master".into()));

        let peeked = cache.peek_or_query("k", LONG, LONG, || panic!("预取过了，不该再问"));
        assert_eq!(peeked.as_deref(), Some("master"));

        let taken = cache.take_or_query("k", LONG, LONG, || panic!("预检不该吃掉结果"));
        assert_eq!(taken.as_deref(), Some("master"));

        let next = cache.take_or_query("k", LONG, LONG, || Some("main".into()));
        assert_eq!(next.as_deref(), Some("main"));
    }

    /// 没赶上预取时，预检查到的答案也留给删除，不多连一次网。
    #[test]
    fn a_peek_keeps_the_answer_it_queried_itself() {
        let cache = PrefetchCache::default();

        let peeked = cache.peek_or_query("k", LONG, LONG, || Some("main".into()));
        assert_eq!(peeked.as_deref(), Some("main"));

        let taken = cache.take_or_query("k", LONG, LONG, || panic!("预检问过了，不该再问"));
        assert_eq!(taken.as_deref(), Some("main"));
    }

    /// 预检失败同样不留痕：之后护栏要自己问，不能沿用。
    #[test]
    fn a_failed_peek_leaves_nothing_behind() {
        let cache = PrefetchCache::default();

        assert_eq!(cache.peek_or_query("k", LONG, LONG, || None), None);

        let taken = cache.take_or_query("k", LONG, LONG, || Some("main".into()));
        assert_eq!(taken.as_deref(), Some("main"));
    }

    /// 过期结果不能因为预检而续命：TTL 没有例外。
    #[test]
    fn a_peek_does_not_use_or_extend_an_expired_result() {
        let cache = PrefetchCache::default();
        with_aged_result(&cache, 40);

        let peeked = cache.peek_or_query("k", TTL, LONG, || Some("new".into()));
        assert_eq!(peeked.as_deref(), Some("new"));

        let taken = cache.take_or_query("k", TTL, LONG, || panic!("预检刚查过"));
        assert_eq!(taken.as_deref(), Some("new"));
    }

    /// 预检看过之后结果的年龄不变：删除时已过期就要当场重问，worktree 删得再久也一样。
    #[test]
    fn a_peek_does_not_reset_the_age_of_a_result() {
        let cache = PrefetchCache::default();
        with_aged_result(&cache, 20);

        let peeked = cache.peek_or_query("k", TTL, LONG, || panic!("结果还没过期"));
        assert_eq!(peeked.as_deref(), Some("old"));

        let started = cache.slot("k").lock().last.as_ref().unwrap().started;
        assert!(started.elapsed() >= Duration::from_secs(20), "年龄被重置了");
    }

    /// 预检也要等路上的预取，不能另发一次。
    #[test]
    fn a_peek_waits_for_the_prefetch_in_flight() {
        let cache = Arc::new(PrefetchCache::default());
        let (prefetch, release) = start_prefetch(&cache, Some("main"));

        let c = cache.clone();
        let peek = thread::spawn(move || {
            c.peek_or_query("k", LONG, LONG, || panic!("等待者不该再问一次"))
        });
        wait_for_waiter(&cache);
        release.send(()).unwrap();

        prefetch.join().unwrap();
        assert_eq!(peek.join().unwrap().as_deref(), Some("main"));
    }

    /// 卡死的预取同样不能把预检拖死，等够上限就自己问。
    #[test]
    fn a_stuck_prefetch_cannot_block_the_peek() {
        let cache = Arc::new(PrefetchCache::default());
        let (prefetch, release) = start_prefetch(&cache, None);

        let got = cache.peek_or_query("k", LONG, Duration::from_millis(50), || Some("main".into()));
        assert_eq!(got.as_deref(), Some("main"));

        release.send(()).unwrap();
        prefetch.join().unwrap();
    }

    /// 反方向：卡住的预取晚回来时，不能盖掉预检等不及自己查到的更新答案。
    #[test]
    fn a_slow_prefetch_does_not_overwrite_a_newer_result() {
        let cache = Arc::new(PrefetchCache::default());
        let (prefetch, release) = start_prefetch(&cache, Some("older"));

        let peeked = cache.peek_or_query("k", LONG, Duration::ZERO, || Some("newer".into()));
        assert_eq!(peeked.as_deref(), Some("newer"));

        release.send(()).unwrap();
        prefetch.join().unwrap();

        let taken = cache.take_or_query("k", LONG, LONG, || panic!("结果还在"));
        assert_eq!(taken.as_deref(), Some("newer"));
    }

    /// 预检查询期间写入的更新答案不能被盖掉。
    #[test]
    fn a_slow_peek_does_not_overwrite_a_newer_result() {
        let cache = PrefetchCache::default();

        let got = cache.peek_or_query("k", LONG, LONG, || {
            cache.prefetch("k", LONG, || Some("newer".into()));
            Some("older".into())
        });
        assert_eq!(got.as_deref(), Some("older"));

        let taken = cache.take_or_query("k", LONG, LONG, || panic!("结果还在"));
        assert_eq!(taken.as_deref(), Some("newer"));
    }
}
