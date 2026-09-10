use std::{fmt::Display, future::Future, time::Duration};

/// Number of retries after the initial attempt. Keep the bootstrap shell policy in sync.
pub(crate) fn max_retries() -> u32 {
    static CACHE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        std::env::var("FOUNDRYUP_MAX_RETRIES").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(5)
    })
}

/// Run the whole operation again after a transient failure, with one bounded retry budget.
pub(crate) async fn retry<T, E: Display, F, Fut>(
    max_retries: u32,
    mut operation: F,
    is_retryable: impl Fn(&E) -> bool,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut delay = Duration::from_secs(1);
    for attempt in 0..=max_retries {
        match operation().await {
            Err(error) if attempt < max_retries && is_retryable(&error) => {
                tracing::warn!(
                    "{error}; retrying in {}s ({}/{max_retries})",
                    delay.as_secs(),
                    attempt + 1,
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(16));
            }
            result => return result,
        }
    }
    unreachable!("the last attempt always returns")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, future::ready};

    #[test]
    fn backoff_is_capped_and_budget_is_bounded() {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap()
            .block_on(async {
                let start = tokio::time::Instant::now();
                let attempts = RefCell::new(Vec::new());
                let result = retry(
                    7,
                    || {
                        attempts.borrow_mut().push(start.elapsed().as_secs());
                        ready(Err::<(), _>("transient"))
                    },
                    |_| true,
                )
                .await;
                assert_eq!(result, Err("transient"));
                assert_eq!(*attempts.borrow(), [0, 1, 3, 7, 15, 31, 47, 63]);
            });
    }

    #[test]
    fn success_permanent_errors_and_zero_budget_stop_retries() {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap()
            .block_on(async {
                for (budget, transient, succeed, expected) in
                    [(5, true, true, 3), (5, false, false, 1), (0, true, false, 1)]
                {
                    let attempts = std::cell::Cell::new(0);
                    let result = retry(
                        budget,
                        || {
                            attempts.set(attempts.get() + 1);
                            ready(if succeed && attempts.get() == 3 {
                                Ok(())
                            } else {
                                Err("failure")
                            })
                        },
                        |_| transient,
                    )
                    .await;
                    assert_eq!(result.is_ok(), succeed);
                    assert_eq!(attempts.get(), expected);
                }
            });
    }
}
