pub fn derive_status(req_status: &str, ticket_status: Option<&str>) -> &'static str {
    if req_status == "CANCELLED" || ticket_status == Some("CANCELLED") {
        "CANCELLED"
    } else if ticket_status == Some("NO_SHOW") {
        "NO_SHOW"
    } else if req_status == "COMPLETED" || ticket_status == Some("COMPLETED") {
        "COMPLETED"
    } else if matches!(req_status, "ACCEPTED" | "ID_CHECKED") || ticket_status == Some("SERVING") {
        "SERVING"
    } else if ticket_status == Some("CALLED") {
        "CALLED"
    } else if ticket_status == Some("WAITING") || req_status == "WAITING" {
        "WAITING"
    } else {
        "QUOTED"
    }
}
