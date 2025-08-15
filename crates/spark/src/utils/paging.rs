use std::fmt::Debug;
use std::future::Future;

const DEFAULT_PAGING_LIMIT: u64 = 100;
const DEFAULT_PAGING_OFFSET: u64 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagingFilter {
    pub offset: u64,
    pub limit: u64,
    pub order: Order,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Order {
    Ascending,
    Descending,
}

impl From<Order> for crate::operator::rpc::spark::Order {
    fn from(value: Order) -> Self {
        match value {
            Order::Ascending => crate::operator::rpc::spark::Order::Ascending,
            Order::Descending => crate::operator::rpc::spark::Order::Descending,
        }
    }
}

impl PagingFilter {
    pub fn new(offset: Option<u64>, limit: Option<u64>, order: Option<Order>) -> Self {
        Self {
            offset: offset.unwrap_or(DEFAULT_PAGING_OFFSET),
            limit: limit.unwrap_or(DEFAULT_PAGING_LIMIT),
            order: order.unwrap_or(Order::Descending),
        }
    }

    pub fn next(&self) -> Self {
        Self {
            offset: self.offset + self.limit,
            limit: self.limit,
            order: self.order.clone(),
        }
    }

    pub fn next_from_offset(&self, offset: i64) -> Option<Self> {
        if offset <= 0 {
            return None;
        }

        Some(self.next())
    }
}

impl Default for PagingFilter {
    fn default() -> Self {
        Self {
            offset: DEFAULT_PAGING_OFFSET,
            limit: DEFAULT_PAGING_LIMIT,
            order: Order::Descending,
        }
    }
}

pub struct PagingResult<T> {
    pub items: Vec<T>,
    pub next: Option<PagingFilter>,
}

impl<T: Debug> Debug for PagingResult<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PagingResult")
            .field("items", &self.items)
            .field("next", &self.next)
            .finish()
    }
}

pub async fn pager<T, F, E>(
    query_fn: impl Fn(PagingFilter) -> F,
    mut paging_filter: PagingFilter,
) -> Result<Vec<T>, E>
where
    F: Future<Output = Result<PagingResult<T>, E>>,
{
    let mut res = Vec::new();
    loop {
        let offset = paging_filter.offset;
        let resp = query_fn(paging_filter).await?;

        // If no items are returned, break the loop
        if resp.items.is_empty() {
            break;
        }

        res.extend(resp.items);

        // If there is no next page, break the loop
        let Some(next) = resp.next else {
            break;
        };

        // If the next page's offset is less than or equal to the current offset, break the loop
        if next.offset <= offset {
            break;
        }
        paging_filter = next;
    }

    Ok(res)
}
#[cfg(test)]
mod tests {
    use super::*;
    use macros::async_test_all;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[async_test_all]
    async fn test_pager_empty_result() {
        let result = pager(
            |_| async {
                Ok::<_, &str>(PagingResult {
                    items: Vec::<u32>::new(),
                    next: None,
                })
            },
            PagingFilter::default(),
        )
        .await;

        assert_eq!(result.unwrap(), Vec::<u32>::new());
    }

    #[async_test_all]
    async fn test_pager_single_page() {
        let result = pager(
            |_| async {
                Ok::<_, &str>(PagingResult {
                    items: vec![1, 2, 3],
                    next: None,
                })
            },
            PagingFilter::default(),
        )
        .await;

        assert_eq!(result.unwrap(), vec![1, 2, 3]);
    }

    #[async_test_all]
    async fn test_pager_error_after_partial_success() {
        let call_count = std::sync::Arc::new(AtomicU64::new(0));

        let result = pager(
            |filter| {
                let filter_clone = filter.clone();
                let counter = call_count.clone();
                async move {
                    let count = counter.fetch_add(1, Ordering::SeqCst);
                    match count {
                        0 => Ok::<_, &str>(PagingResult {
                            items: vec![1, 2, 3],
                            next: Some(filter_clone.next()),
                        }),
                        _ => Err("failed on second page"),
                    }
                }
            },
            PagingFilter::default(),
        )
        .await;

        assert_eq!(result.unwrap_err(), "failed on second page");
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[async_test_all]
    async fn test_pager_multiple_pages() {
        let call_count = std::sync::Arc::new(AtomicU64::new(0));

        let result = pager(
            |filter| {
                let filter_clone = filter.clone();
                let counter = call_count.clone();
                async move {
                    let count = counter.fetch_add(1, Ordering::SeqCst);
                    match count {
                        0 => Ok::<_, &str>(PagingResult {
                            items: vec![1, 2, 3],
                            next: Some(filter_clone.next()),
                        }),
                        1 => Ok::<_, &str>(PagingResult {
                            items: vec![4, 5, 6],
                            next: Some(filter_clone.next()),
                        }),
                        _ => Ok::<_, &str>(PagingResult {
                            items: vec![7, 8, 9],
                            next: None,
                        }),
                    }
                }
            },
            PagingFilter::default(),
        )
        .await;

        assert_eq!(result.unwrap(), vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[async_test_all]
    async fn test_pager_with_custom_filter() {
        let custom_filter = PagingFilter::new(Some(10), Some(5), None);
        let expected_offsets = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

        let result = pager(
            |filter| {
                let filter_clone = filter.clone();
                let offsets = expected_offsets.clone();
                async move {
                    offsets.lock().unwrap().push(filter_clone.offset);
                    if filter_clone.offset < 20 {
                        Ok::<_, &str>(PagingResult {
                            items: vec![filter_clone.offset as u32],
                            next: Some(filter_clone.next()),
                        })
                    } else {
                        Ok::<_, &str>(PagingResult {
                            items: vec![filter_clone.offset as u32],
                            next: None,
                        })
                    }
                }
            },
            custom_filter,
        )
        .await;

        assert_eq!(result.unwrap(), vec![10, 15, 20]);
        assert_eq!(*expected_offsets.lock().unwrap(), vec![10, 15, 20]);
    }

    #[async_test_all]
    async fn test_pager_error_propagation() {
        let result = pager(
            |_| async { Err::<PagingResult<u32>, _>("test error") },
            PagingFilter::default(),
        )
        .await;

        assert_eq!(result.unwrap_err(), "test error");
    }

    #[async_test_all]
    async fn test_pager_stops_on_invalid_next_offset() {
        let call_count = std::sync::Arc::new(AtomicU64::new(0));

        let result = pager(
            |filter| {
                let filter_clone = filter.clone();
                let counter = call_count.clone();
                async move {
                    let count = counter.fetch_add(1, Ordering::SeqCst);
                    if count == 0 {
                        // Return a next page with the same offset (should cause pager to stop)
                        Ok::<_, &str>(PagingResult {
                            items: vec![1, 2, 3],
                            next: Some(PagingFilter {
                                offset: filter_clone.offset,
                                limit: filter_clone.limit,
                                order: filter_clone.order.clone(),
                            }),
                        })
                    } else {
                        // This should never be called
                        Ok::<_, &str>(PagingResult {
                            items: vec![4, 5, 6],
                            next: None,
                        })
                    }
                }
            },
            PagingFilter::default(),
        )
        .await;

        assert_eq!(result.unwrap(), vec![1, 2, 3]);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[async_test_all]
    async fn test_pager_stops_on_empty_items_with_next() {
        let call_count = std::sync::Arc::new(AtomicU64::new(0));

        let result = pager(
            |filter| {
                let filter_clone = filter.clone();
                let counter = call_count.clone();
                async move {
                    let count = counter.fetch_add(1, Ordering::SeqCst);
                    if count == 0 {
                        // Return empty items but with a next page - should break the loop
                        Ok::<_, &str>(PagingResult {
                            items: vec![],
                            next: Some(filter_clone.next()),
                        })
                    } else {
                        // This should never be called
                        Ok::<_, &str>(PagingResult {
                            items: vec![4, 5, 6],
                            next: None,
                        })
                    }
                }
            },
            PagingFilter::default(),
        )
        .await;

        // Result should be empty and call_count should be 1
        assert_eq!(result.unwrap(), Vec::<i32>::new());
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }
}
