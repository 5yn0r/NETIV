//! Challenges, scoring, progression, parcours d'apprentissage, mode enseignant.

pub mod builtin;
pub mod challenge;
pub mod course;
pub mod faultspec;
pub mod generate;
pub mod learn;
pub mod profile;
pub mod range;
pub mod run;
pub mod tutor;

pub use challenge::{Built, Challenge, ChallengeError, Objective, ServiceRef, Solution};
pub use course::{Course, CourseItem, Grade, ItemKind};
pub use faultspec::FaultSpec;
pub use learn::{LearningPath, Lesson};
pub use profile::{BADGES, Badge, Profile, Recorded};
pub use range::{RangeRun, RangeScenario};
pub use run::{AnswerOutcome, ChallengeRun, CheckOutcome, RunError, Score};
