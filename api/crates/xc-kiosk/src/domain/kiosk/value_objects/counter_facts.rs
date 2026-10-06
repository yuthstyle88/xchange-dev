use xc_kernel::money::Rate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterFacts {
    pub open: bool,
    pub rate: Option<Rate>,
}
