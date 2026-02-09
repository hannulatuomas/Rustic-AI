#[derive(Debug, Clone)]
pub struct Basket {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct SubBasket {
    pub id: String,
    pub basket_id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct BasketMembership {
    pub item_id: String,
    pub basket_id: String,
    pub sub_basket_id: Option<String>,
}
