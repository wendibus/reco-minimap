import tempfile
import unittest
from pathlib import Path
from evaluate_analysis import assignment, evaluate, read_frames, event_metrics


class EvaluationTests(unittest.TestCase):
    def test_global_matching_beats_nearest_first(self):
        self.assertEqual(assignment([[1, 2, 10, 10], [1.1, 9, 10, 10]]), [1, 0])

    def test_missing_frame_counts_as_miss_not_silent_exclusion(self):
        labels = [{"t": 2, "players": [{"id": "h4", "team": "home", "number": 4, "x": 3, "y": 4}],
                   "ball": {"state": "held", "x": 3, "y": 4, "holder_player_id": "h4"}}]
        result = evaluate([], labels)
        self.assertEqual(result["matched_frames"], 0)
        self.assertEqual(result["player_recall"], 0)
        self.assertEqual(result["identity_accuracy"], 0)
        self.assertEqual(result["possession_accuracy"], 0)
        self.assertEqual(result["ball_position_recall"], 0)

    def test_numbers_possession_and_switches_use_person_labels(self):
        labels = [{"t": t, "players": [{"id": "h4", "team": "home", "number": 4, "x": 3, "y": 4}],
                   "ball": {"state": "held", "holder_player_id": "h4"}} for t in (0, .1)]
        predictions = [{"t": t, "players": [{"id": i, "segment": 0, "team": "home", "jersey": 4,
                                            "state": "visible", "x": 3, "y": 4}],
                        "ball": {"state": "held", "holder": i}} for t, i in ((0, 1), (.1, 2))]
        result = evaluate(predictions, labels)
        self.assertEqual(result["identity_accuracy"], 1)
        self.assertEqual(result["possession_accuracy"], 1)
        self.assertEqual(result["sampled_id_switches"], 1)

    def test_negative_ball_and_empty_player_labels_count_false_positives(self):
        pred = [{"t": 0, "players": [{"id": 1, "x": 2, "y": 3}], "ball": {"x": 2, "y": 3}}]
        result = evaluate(pred, [{"t": 0, "players": [], "ball": {"present": False}}])
        self.assertEqual(result["false_ball_positions"], 1)
        self.assertEqual(result["extra_players"], 1)
        self.assertIsNone(result["identity_accuracy"])

    def test_events_are_one_to_one_and_only_scored_in_labelled_windows(self):
        predictions=[{"t":2,"events":[{"t":2,"kind":"shot_three"},{"t":2.2,"kind":"shot_three"}]},
                     {"t":20,"events":[{"t":20,"kind":"shot_two"}]}]
        reference=[{"t":2,"event_window":[0,5],"events":[{"kind":"shot_three"}]}]
        result=event_metrics(predictions,reference)
        self.assertEqual(result["correct_events"],1)
        self.assertEqual(result["false_events"],1)
        self.assertEqual(result["event_recall"],1)
        self.assertEqual(result["event_precision"],0.5)

    def test_duplicate_timestamps_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/"frames.jsonl"
            path.write_text('{"t":1}\n{"t":1}\n')
            with self.assertRaises(ValueError):
                read_frames(path)


if __name__ == "__main__":
    unittest.main()
