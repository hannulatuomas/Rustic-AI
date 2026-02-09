use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct TopicTracker {
    current_topics: Vec<String>,
    last_updated: Option<Instant>,
    min_interval: Duration,
    similarity_threshold: f64,
}

impl TopicTracker {
    pub fn new(min_interval_secs: u64, similarity_threshold: f64) -> Self {
        Self {
            current_topics: Vec::new(),
            last_updated: None,
            min_interval: Duration::from_secs(min_interval_secs),
            similarity_threshold,
        }
    }

    pub fn current_topics(&self) -> &[String] {
        &self.current_topics
    }

    pub fn should_reinfer(&self) -> bool {
        match self.last_updated {
            None => true,
            Some(last_updated) => last_updated.elapsed() >= self.min_interval,
        }
    }

    pub fn should_accept_update(&self, new_topics: &[String]) -> bool {
        let similarity = jaccard_similarity(&self.current_topics, new_topics);
        similarity <= self.similarity_threshold
    }

    pub fn update_topics(&mut self, new_topics: Vec<String>) {
        self.current_topics = normalize_topics(new_topics);
        self.last_updated = Some(Instant::now());
    }
}

fn normalize_topics(topics: Vec<String>) -> Vec<String> {
    let mut unique = BTreeSet::new();
    for topic in topics {
        let trimmed = topic.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            continue;
        }
        unique.insert(trimmed);
    }
    unique.into_iter().collect()
}

fn jaccard_similarity(existing: &[String], incoming: &[String]) -> f64 {
    let existing_set = existing
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let incoming_set = incoming
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();

    if existing_set.is_empty() && incoming_set.is_empty() {
        return 1.0;
    }

    let intersection = existing_set.intersection(&incoming_set).count() as f64;
    let union = existing_set.union(&incoming_set).count() as f64;
    if union == 0.0 {
        1.0
    } else {
        intersection / union
    }
}
