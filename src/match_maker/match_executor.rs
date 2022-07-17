use diesel::SqliteConnection;

use crate::{
    backend::models::{match_model::Match, submission_model::Submission, turn_model::Turn},
    external_related::repo_updater::get_issue_url,
    game::{
        board::{board_to_string, Tile},
        entry_point::initialize_game_session,
        game::{ErrorType, GameResult},
        player::PlayerType,
    },
};

use super::mmr::{calculate_mmr, MMR};

pub(crate) struct MatchReport {
    pub(crate) report: String,
    pub(crate) issue_number: i32,
}

pub(super) fn execute_match_queue(
    conn: &SqliteConnection,
    match_queue: Vec<(Submission, Submission)>,
) -> Vec<(MatchReport, MatchReport)> {
    // TODO if a submission is disqualified and are queued to play multiple
    // TODO games, those games should be skipped.
    let mut round_reports: Vec<(MatchReport, MatchReport)> = Vec::new();
    for i in 0..match_queue.len() {
        let MatchReturn {
            mut p1,
            mut p2,
            winner_id,
            loser_id,
            error_msg,
            error_fault,
            turns,
        } = start_match(match_queue[i].clone());

        let report = create_report_text(
            error_msg.clone(),
            error_fault.clone(),
            p1.id.clone(),
            p1.issue_number,
            p2.id.clone(),
            p2.issue_number,
        );
        round_reports.push((
            MatchReport {
                report: report.0,
                issue_number: p1.issue_number,
            },
            MatchReport {
                report: report.1,
                issue_number: p2.issue_number,
            },
        ));

        // If the new challenger has a part in the error
        // we disqualify it directly here
        if error_msg.is_some()
            && error_fault.is_some()
            && error_fault.unwrap() == PlayerType::Flipped
            || winner_id.is_none()
            || loser_id.is_none()
        {
            continue;
        }

        let winner_id = winner_id.unwrap();
        let loser_id = loser_id.unwrap();

        let (p1_new_mmr, p2_new_mmr) = calculate_mmr(
            MMR {
                rating: p1.mmr,
                matches_played: p1.matches_played,
            },
            MMR {
                rating: p2.mmr,
                matches_played: p2.matches_played,
            },
            p1.id == winner_id,
            0.5,
        );

        // Assign new mmr
        p1.mmr = p1_new_mmr;
        p2.mmr = p2_new_mmr;
        // Increment matches played
        p1.matches_played += 1;
        p2.matches_played += 1;

        // TODO create a queue and save all submissions in the end
        // TODO this way if the same submission is playing twice it only needs to be saved once
        p1.save(conn);
        p2.save(conn);

        // We never save a match if it wasn't successful
        // If we get to this point we know there
        // were no errors

        // If there are errors, then we stop the match-making process
        // This is because the submitted bot is obviously not working
        // and should therefore not be matchmaked against future bots
        match Match::create(&winner_id, &loser_id, conn) {
            Some(match_record) => {
                // Generate turns
                let mut turn_index = 1;
                for turn in turns {
                    Turn::create(&match_record.id, turn_index, &board_to_string(turn), conn);
                    turn_index += 1;
                }
            }
            None => {
                println!("Internal error, could not create match");
            }
        }
    }
    return round_reports;
}

struct MatchReturn {
    p1: Submission,
    p2: Submission,
    winner_id: Option<String>,
    loser_id: Option<String>,
    turns: Vec<Vec<Tile>>,
    error_msg: Option<String>,
    error_fault: Option<PlayerType>,
}

fn start_match(players: (Submission, Submission)) -> MatchReturn {
    let mut error_fault: Option<PlayerType> = None;
    let mut error_msg: Option<String> = None;

    let (mut p1, mut p2) = players;

    let (result, turns) = initialize_game_session(&p1.script, &p2.script);
    let winner: Option<String>;
    let loser: Option<String>;

    let p1_id = p1.id.clone();
    let p2_id = p2.id.clone();

    match result {
        GameResult::PlayerOneWon => {
            p1.wins += 1;
            winner = Some(p1_id);
            loser = Some(p2_id);
        }
        GameResult::PlayerTwoWon => {
            p2.wins += 1;
            winner = Some(p2_id);
            loser = Some(p1_id);
        }
        GameResult::Error(error) => {
            // If an error occur, it's no longer a matter
            // of who is the winner, it's rather a matter
            // of who is the going to be disqualified.
            winner = None;
            loser = None;

            match error {
                ErrorType::GameError { reason, fault }
                | ErrorType::RuntimeError { reason, fault } => {
                    error_fault = fault;
                    error_msg = Some(reason);
                    p2.disqualified = 1;
                }
                ErrorType::TurnTimeout { fault } => {
                    error_fault = fault;
                    error_msg = Some("Turn timeout".to_string());
                    p2.disqualified = 1;
                }
                ErrorType::GameDeadlock => {
                    error_msg = Some("Deadlock, both bots failed".to_string());
                    p2.disqualified = 1;
                }
            }

            // Challenger is always the flipped player
            match error_fault {
                Some(PlayerType::Regular) => {
                    println!("Disq reg");
                    p2.disqualified = 1;
                }
                Some(PlayerType::Flipped) => {
                    println!("Disq flip");
                    p1.disqualified = 1;
                }
                None => {
                    println!("Disq both");
                    // Both are disqualified
                    p1.disqualified = 1;
                    p2.disqualified = 1;
                }
            }
        }
    }

    return MatchReturn {
        p1,
        p2,
        winner_id: winner,
        loser_id: loser,
        turns,
        error_msg,
        error_fault,
    };
}

/// Returns two reports, one for p1 and one for p2
fn create_report_text(
    error_msg: Option<String>,
    fault: Option<PlayerType>,
    p1: String,
    p1_issue_number: i32,
    p2: String,
    p2_issue_number: i32,
    winner_id: String,
) -> (String, String) {
    let p1_issue = get_issue_url(p1_issue_number);
    let p2_issue = get_issue_url(p2_issue_number);

    match error_msg {
        Some(error_msg) => {
            return (
                get_error_report(
                    format!("[{}]({})", p2, p2_issue),
                    error_msg.clone(),
                    fault.clone(),
                ),
                get_error_report(format!("[{}]({})", p1, p1_issue), error_msg, fault),
            );
        }
        None => (
            format!(
                "[{}] Opponent: [{}]({})",
                if winner_id == p1 { "WIN" } else { "LOSS" },
                p2,
                p2_issue
            ),
            format!(
                "[{}] Opponent: [{}]({})",
                if winner_id == p1 { "WIN" } else { "LOSS" },
                p1,
                p1_issue
            ),
        ),
    }
}

fn get_error_report(
    opponent_issue_link: String,
    error_msg: String,
    fault: Option<PlayerType>,
) -> String {
    format!(
        "[FAIL] Opponent: {}, Error: {}, {}",
        opponent_issue_link,
        error_msg,
        match fault {
            Some(PlayerType::Flipped) => format!("submission has been disqualified"),
            Some(PlayerType::Regular) => format!("opponent has been disqualified"),
            None => format!("both players have been disqualifed"),
        }
    )
}
