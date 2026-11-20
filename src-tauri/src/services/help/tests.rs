use super::*;

/// A build with every feature (a trial gateway and a public relay set)
const ALL: Offered = Offered {
    free_trial: true,
    public_relay: true,
};

/// A build with neither (no URLs set, as 0.2.0 ships)
const NEITHER: Offered = Offered {
    free_trial: false,
    public_relay: false,
};

fn best(question: &str) -> Option<String> {
    best_in(question, ALL)
}

fn best_in(question: &str, offered: Offered) -> Option<String> {
    answer_for(question, HelpReason::NoModel, None, offered)
        .topic
        .map(|topic| topic.id)
}

#[test]
fn the_guide_file_is_complete() {
    let topics = &guide().topics;
    assert!(topics.len() >= 10);
    let mut ids: Vec<&str> = topics.iter().map(|t| t.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), topics.len(), "topic IDs are unique");
    for topic in topics {
        for language in ["en", "fr", "ar", "ja"] {
            assert!(
                topic.keywords.get(language).is_some_and(|k| !k.is_empty()),
                "{} has {language} keywords",
                topic.id
            );
        }
        assert!(!topic.links.is_empty(), "{} links somewhere", topic.id);
        for link in &topic.links {
            let known = link.target.starts_with("url:https://")
                || link.target.starts_with("settings")
                || link.target.starts_with("network:")
                || ["agents", "library", "automations", "inbox", "home"]
                    .contains(&link.target.as_str());
            assert!(known, "{}: {}", topic.id, link.target);
        }
    }
}

#[test]
fn common_questions_find_their_topic() {
    let cases = [
        ("How do I install Docker?", "install-docker"),
        ("Agents need Docker to act — what now?", "install-docker"),
        ("How do I connect a model?", "connect-model"),
        ("Where do I paste my OpenAI API key?", "api-key"),
        ("Can I sign in with Claude Code instead?", "claude-sign-in"),
        ("I want to run a local model with Ollama", "ollama"),
        ("My free tokens are used up", "free-trial"),
        ("How do I join my team's relay?", "join-relay"),
        ("How do I join the public relay?", "public-relay"),
        ("What is an agent?", "agents"),
        ("What are groups for?", "groups"),
        ("What's in the Inbox?", "inbox"),
        ("Why does Nandi want to get to know me?", "know-me"),
        ("What is Tell Nandi about you?", "know-me"),
        ("Where is everything?", "find-your-way"),
    ];
    for (question, topic) in cases {
        assert_eq!(best(question).as_deref(), Some(topic), "{question}");
    }
}

#[test]
fn questions_in_every_language_match() {
    assert_eq!(
        best("Comment installer Docker ?").as_deref(),
        Some("install-docker")
    );
    // Accents optional
    assert_eq!(best("ou ajouter ma cle API").as_deref(), Some("api-key"));
    assert_eq!(
        best("Comment rejoindre un relais ?").as_deref(),
        Some("join-relay")
    );
    assert_eq!(best("كيف أثبت دوكر؟").as_deref(), Some("install-docker"));
    assert_eq!(best("ما هو الوكيل؟").as_deref(), Some("agents"));
    assert_eq!(
        best("Dockerをインストールするには？").as_deref(),
        Some("install-docker")
    );
    assert_eq!(best("グループとは何ですか").as_deref(), Some("groups"));
}

#[test]
fn keywords_match_whole_words_only() {
    assert!(contains_keyword("my keys are gone", "key"));
    assert!(contains_keyword("add a key.", "key"));
    assert!(!contains_keyword("keyboard shortcuts", "key"));
    assert!(!contains_keyword("monkey", "key"));
    assert!(!contains_keyword("a reagent", "agent"));
}

#[test]
fn nothing_matching_offers_every_topic() {
    let answer = answer_for(
        "Bake me a cake",
        HelpReason::ModelFailed,
        Some("offline".into()),
        ALL,
    );
    assert_eq!(answer.topic, None);
    assert_eq!(answer.related.len(), guide().topics.len());
    assert_eq!(answer.reason, HelpReason::ModelFailed);
    assert_eq!(answer.detail.as_deref(), Some("offline"));
}

#[test]
fn an_answer_suggests_the_next_matches() {
    // Docker first; the free trial and models come along
    let answer = answer_for(
        "install docker and connect a model with free tokens",
        HelpReason::NoModel,
        None,
        ALL,
    );
    assert_eq!(answer.topic.unwrap().id, "install-docker");
    let related: Vec<String> = answer.related.into_iter().map(|t| t.id).collect();
    assert!(related.contains(&"connect-model".to_string()));
    assert!(related.contains(&"free-trial".to_string()));
    assert!(related.len() <= MAX_RELATED);
}

#[test]
fn a_build_without_a_feature_never_points_to_it() {
    let ids = |offered| -> Vec<String> { topics_for(offered).into_iter().map(|t| t.id).collect() };
    assert!(ids(ALL).contains(&"free-trial".to_string()));
    assert!(ids(ALL).contains(&"public-relay".to_string()));
    let neither = ids(NEITHER);
    assert!(!neither.contains(&"free-trial".to_string()));
    assert!(!neither.contains(&"public-relay".to_string()));
    assert!(neither.contains(&"join-relay".to_string()));
    // Asked about them, the guide answers with what this Nandi has
    assert_ne!(
        best_in("My free tokens are used up", NEITHER).as_deref(),
        Some("free-trial")
    );
    assert_eq!(
        best_in("How do I join the public relay?", NEITHER).as_deref(),
        Some("join-relay")
    );
    let cake = answer_for("Bake me a cake", HelpReason::NoModel, None, NEITHER);
    assert_eq!(cake.related.len(), neither.len());
    // One feature at a time
    let trial_only = Offered {
        free_trial: true,
        public_relay: false,
    };
    assert!(ids(trial_only).contains(&"free-trial".to_string()));
    assert!(!ids(trial_only).contains(&"public-relay".to_string()));
}
