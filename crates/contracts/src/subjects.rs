use orderflow_domain::Market;

pub const TYPE_HEADER: &str = "Of-Type";

const VERSION: &str = "v1";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("namespace `{0}` must be non-empty lowercase ASCII letters, digits or `_`")]
pub struct NamespaceError(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Namespace {
    prefix: String,
}

impl Default for Namespace {
    fn default() -> Self {
        Self::production()
    }
}

impl Namespace {
    pub fn production() -> Self {
        Self {
            prefix: "of".into(),
        }
    }

    pub fn new(prefix: &str) -> Result<Self, NamespaceError> {
        let valid = !prefix.is_empty()
            && prefix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valid {
            return Err(NamespaceError(prefix.to_owned()));
        }
        Ok(Self {
            prefix: prefix.to_owned(),
        })
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub fn trades(&self, market: &Market) -> String {
        self.market_subject("trade", market)
    }

    pub fn gaps(&self, market: &Market) -> String {
        self.market_subject("gap", market)
    }

    pub fn all_trades(&self) -> String {
        format!("{}.{VERSION}.trade.>", self.prefix)
    }

    pub fn all_gaps(&self) -> String {
        format!("{}.{VERSION}.gap.>", self.prefix)
    }

    pub fn bubbles(&self, market: &Market) -> String {
        self.market_subject("bubble", market)
    }

    pub fn all_bubbles(&self) -> String {
        format!("{}.{VERSION}.bubble.>", self.prefix)
    }

    pub fn book(&self, market: &Market) -> String {
        self.market_subject("book", market)
    }

    pub fn heatmap(&self, market: &Market) -> String {
        self.market_subject("heatmap", market)
    }

    pub fn record_stream(&self) -> String {
        format!(
            "{}_RECORD_{}",
            self.prefix.to_uppercase(),
            VERSION.to_uppercase()
        )
    }

    pub fn record_stream_subjects(&self) -> Vec<String> {
        vec![self.all_trades(), self.all_gaps(), self.all_bubbles()]
    }

    pub fn kv_bucket(&self, purpose: &str) -> String {
        format!("{}_{purpose}_{VERSION}", self.prefix)
    }

    fn market_subject(&self, kind: &str, market: &Market) -> String {
        format!(
            "{}.{VERSION}.{kind}.{}.{}.{}",
            self.prefix,
            market.venue().as_str(),
            market.market_type().as_str(),
            market.native_name()
        )
    }
}

pub fn type_name<M: prost::Name>() -> String {
    M::full_name()
}
