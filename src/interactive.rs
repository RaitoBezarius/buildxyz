/// Request types between FUSE thread and UI thread
enum Request {
    /// An interactive search request for the given path to the UI thread
    InteractiveSearch(String),
}

/// Response types between UI thread and FUSE thread
enum Response {
    PackageSuggestion(String),
}
