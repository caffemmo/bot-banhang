use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Stat {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardStats {
    pub total_revenue: i64,
    pub today_revenue: i64,
    pub pending_orders: i64,
    pub completed_orders: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardAggregatedStats {
    pub total_users: i64,
    pub active_products: i64,
    pub active_plans: i64,
    pub pending_orders: i64,
    pub lifetime_revenue: i64,
}
