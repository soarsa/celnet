"""Unit tests for the spec-extract v0 heuristics.

Run via: scripts/spec-extract.sh --test
       or: scripts/spec-extract/.venv/bin/python -m unittest discover scripts/spec-extract
"""
import unittest

from extract import (
    APP_MESSAGES,
    SESSION_MESSAGES,
    detect_message_types,
    parse_field_table,
    render_xml,
)


class DetectMessageTypesTest(unittest.TestCase):
    def test_wire_form_is_detected(self):
        text = "When the client sends 35=D the gateway responds with 35=8."
        self.assertEqual(detect_message_types([text]), {"D", "8"})

    def test_msgtype_equals_form(self):
        text = "MsgType = D (NewOrderSingle)\nMsgType (35) = 8"
        self.assertEqual(detect_message_types([text]), {"D", "8"})

    def test_message_type_colon_form(self):
        text = "Message Type: V - MarketDataRequest\nMessage Type: W - Snapshot"
        self.assertEqual(detect_message_types([text]), {"V", "W"})

    def test_two_letter_msg_types(self):
        # AJ (QuoteResponse) is real FIX 4.4; AE (TradeCaptureReport) too.
        text = "35=AJ is QuoteResponse. 35=AE is the trade capture report."
        self.assertEqual(detect_message_types([text]), {"AJ", "AE"})

    def test_unknown_msg_types_are_ignored(self):
        # 35=ZZ isn't in our catalogue → drop it.
        text = "35=ZZ is some vendor-custom thing. 35=D is real."
        self.assertEqual(detect_message_types([text]), {"D"})

    def test_no_msg_types(self):
        text = "This PDF talks about FIX but never names a message type."
        self.assertEqual(detect_message_types([text]), set())

    def test_multiple_pages_aggregate(self):
        pages = ["35=D appears here.", "35=8 appears on another page.", "no msgtypes."]
        self.assertEqual(detect_message_types(pages), {"D", "8"})


class RenderXmlTest(unittest.TestCase):
    def test_envelope_skeleton_is_always_emitted(self):
        xml = render_xml(vendor="Acme", source_pdf="x.pdf", detected=set())
        self.assertIn('<fix major="4" minor="4" type="FIX">', xml)
        self.assertIn("<header>", xml)
        self.assertIn("<trailer>", xml)
        self.assertIn('<field name="CheckSum" required="Y"/>', xml)

    def test_fallback_note_appears_when_no_detections(self):
        xml = render_xml(vendor="Acme", source_pdf="x.pdf", detected=set())
        self.assertIn("FALLBACK", xml)
        self.assertIn("FIX 4.4 starter catalogue", xml)
        # The starter catalogue includes the common primary message types so
        # downstream conformance tests assert correctly even on fallback.
        self.assertIn('msgtype="D"', xml)
        self.assertIn('msgtype="V"', xml)
        self.assertIn('msgtype="R"', xml)

    def test_session_messages_always_present(self):
        xml = render_xml(vendor="Acme", source_pdf="x.pdf", detected=set())
        for m in SESSION_MESSAGES:
            self.assertIn(f'msgtype="{m.msg_type}"', xml,
                f"session message {m.name} ({m.msg_type}) should always be emitted")

    def test_detected_app_messages_appear(self):
        xml = render_xml(vendor="Acme", source_pdf="x.pdf", detected={"D", "8"})
        self.assertIn('msgtype="D"', xml)
        self.assertIn('msgtype="8"', xml)
        self.assertIn('name="NewOrderSingle"', xml)
        self.assertIn('name="ExecutionReport"', xml)
        # Fallback note should NOT be present when something was detected.
        self.assertNotIn("FALLBACK", xml)

    def test_vendor_and_source_in_header_comment(self):
        xml = render_xml(vendor="Citi RFS", source_pdf="CitiFX.pdf", detected={"D"})
        self.assertIn("Citi RFS", xml)
        self.assertIn("CitiFX.pdf", xml)

    def test_unknown_detected_msg_type_is_silently_skipped(self):
        # Callers can inject anything; unknown msg types get silently dropped
        # from the rendered XML. (`detect_message_types` filters too, but
        # `render_xml` shouldn't trust its caller — table extraction could
        # surface a wider set as the heuristic evolves.)
        xml = render_xml(
            vendor="X", source_pdf="x.pdf", detected={"BOGUS"},
        )
        # No msgtype="BOGUS" appears in the output.
        self.assertNotIn('msgtype="BOGUS"', xml)
        # Session messages are still emitted regardless.
        self.assertIn('msgtype="A"', xml)


class ParseFieldTableTest(unittest.TestCase):
    def test_clean_table_yields_msg_type_and_fields(self):
        # The minimal shape: header + msgtype binding row + a few field rows.
        table = [
            ["tag", "field name", "required", "comments"],
            ["", "Standard Header", "", "MsgType tag 35 = D"],
            ["11", "ClOrdID", "Y", "Client order id"],
            ["55", "Symbol", "Y", "Instrument"],
            ["44", "Price", "N", "Limit price"],
        ]
        msg_type, fields = parse_field_table(table)
        self.assertEqual(msg_type, "D")
        self.assertEqual(fields, [("ClOrdID", True), ("Symbol", True), ("Price", False)])

    def test_split_columns_with_empty_separators(self):
        # pdfplumber often splits one logical column across several cells.
        # The parser should pick the first non-empty cell in each header range.
        table = [
            ["tag", "", "field name", "", "required", "", "comments"],
            ["", "", "Standard Header", "", "", "", "MsgType = V"],
            ["262", "", "MDReqID", "", "Y", "", "Request ID"],
            ["263", "", "SubscriptionRequestType", "", "Y", "", "Sub type"],
        ]
        msg_type, fields = parse_field_table(table)
        self.assertEqual(msg_type, "V")
        self.assertEqual(fields, [("MDReqID", True), ("SubscriptionRequestType", True)])

    def test_reqd_apostrophe_form(self):
        # Some specs use "req'd" instead of "required".
        table = [
            ["tag", "field name", "req'd", "comments"],
            ["", "Standard Header", "", "35=R"],
            ["131", "QuoteReqID", "Y", "Unique"],
        ]
        msg_type, fields = parse_field_table(table)
        self.assertEqual(msg_type, "R")
        self.assertEqual(fields, [("QuoteReqID", True)])

    def test_m_means_required(self):
        # Some specs use M (Mandatory) instead of Y.
        table = [
            ["tag", "field name", "m", "comments"],
            ["", "Standard Header", "", "35=D"],
            ["11", "ClOrdID", "M", "Required"],
            ["44", "Price", "", "Optional"],
        ]
        msg_type, fields = parse_field_table(table)
        self.assertEqual(msg_type, "D")
        self.assertEqual(fields, [("ClOrdID", True), ("Price", False)])

    def test_standard_header_and_trailer_rows_are_dropped(self):
        # Vendor specs include these as reference rows in every message table;
        # our XML re-emits them from the canonical <header>/<trailer>, so we
        # must not duplicate them inside <messages>.
        table = [
            ["tag", "field name", "required", "comments"],
            ["", "Standard Header", "Y", "MsgType = D"],
            ["11", "ClOrdID", "Y", ""],
            ["", "Standard Trailer", "Y", ""],
        ]
        _, fields = parse_field_table(table)
        self.assertEqual([n for n, _ in fields], ["ClOrdID"])

    def test_duplicate_field_rows_collapse(self):
        # Repeating-group member fields sometimes get listed twice (once as
        # the group definition, once as the group member). Take the first.
        table = [
            ["tag", "field name", "required", "comments"],
            ["", "Standard Header", "", "35=D"],
            ["453", "NoPartyIDs", "N", "Count of party ids"],
            ["448", "PartyID", "N", "Party id"],
            ["448", "PartyID", "N", "Party id (group member)"],
        ]
        _, fields = parse_field_table(table)
        self.assertEqual([n for n, _ in fields], ["NoPartyIDs", "PartyID"])

    def test_table_without_tag_column_returns_nothing(self):
        # If it's not a field table, leave it alone.
        table = [
            ["product", "currency", "supported"],
            ["EUR/USD", "Y", "Y"],
        ]
        msg_type, fields = parse_field_table(table)
        self.assertIsNone(msg_type)
        self.assertEqual(fields, [])

    def test_empty_or_tiny_table_handled(self):
        self.assertEqual(parse_field_table([]), (None, []))
        self.assertEqual(parse_field_table([["only one row"]]), (None, []))

    def test_msg_type_from_caption_above_table(self):
        # pdfplumber sometimes pulls a caption into a single-cell row before
        # the header. The parser scans every row's joined text for MsgType.
        table = [
            ["Message Type: D - NewOrderSingle"],
            ["tag", "field name", "required", "comments"],
            ["11", "ClOrdID", "Y", ""],
        ]
        msg_type, _ = parse_field_table(table)
        # First row has no "tag" header, so column detection runs on row[1].
        # parse_field_table currently expects table[0] to be the header — so
        # this caption-as-first-row case is NOT yet handled. Document it.
        self.assertIsNone(msg_type)  # known v1 limitation; fix in v1.1.


class RenderXmlWithExtractedFieldsTest(unittest.TestCase):
    def test_extracted_fields_replace_catalogue_defaults(self):
        # When extracted_tables provides a field list for a detected msg type,
        # render_xml uses those instead of the catalogue's standard fields.
        xml = render_xml(
            vendor="Acme",
            source_pdf="x.pdf",
            detected={"D"},
            extracted_tables={"D": [("ClOrdID", True), ("CustomTag", False)]},
        )
        # Standard catalogue would include Symbol/Side/OrderQty; the extracted
        # list replaces it.
        self.assertIn('name="ClOrdID"', xml)
        self.assertIn('name="CustomTag"', xml)
        self.assertNotIn('name="Symbol"', xml)

    def test_session_messages_can_use_extracted_fields(self):
        # If a vendor's Logon table is extractable, use it; otherwise the
        # catalogue's standard Logon fields apply.
        xml = render_xml(
            vendor="Acme",
            source_pdf="x.pdf",
            detected=set(),
            extracted_tables={"A": [("EncryptMethod", True), ("VendorAuthToken", True)]},
        )
        self.assertIn('name="VendorAuthToken"', xml)

    def test_no_extracted_tables_uses_catalogue_defaults(self):
        # Backwards-compatible: callers that don't pass extracted_tables
        # behave exactly like v0.
        xml_v0 = render_xml(vendor="A", source_pdf="x.pdf", detected={"D"})
        xml_v1_empty = render_xml(
            vendor="A", source_pdf="x.pdf",
            detected={"D"}, extracted_tables={},
        )
        # Both emit the catalogue's standard NewOrderSingle (Symbol included).
        self.assertIn('name="Symbol"', xml_v0)
        self.assertIn('name="Symbol"', xml_v1_empty)

    def test_summary_comment_lists_extracted_msg_types(self):
        xml = render_xml(
            vendor="A", source_pdf="x.pdf",
            detected={"D", "V"},
            extracted_tables={"D": [("ClOrdID", True)]},  # V has no extraction
        )
        # Summary line: extracted for D only.
        self.assertIn("Field tables extracted for: D", xml)


class CatalogueIntegrityTest(unittest.TestCase):
    """Sanity checks on the message catalogues themselves."""

    def test_session_and_app_messages_dont_collide(self):
        session_types = {m.msg_type for m in SESSION_MESSAGES}
        app_types = set(APP_MESSAGES.keys())
        self.assertEqual(session_types & app_types, set(),
            "session and app messages must not share MsgType codes")

    def test_app_messages_dict_keys_match_msg_types(self):
        for key, defn in APP_MESSAGES.items():
            self.assertEqual(key, defn.msg_type)

    def test_every_message_has_at_least_one_field(self):
        # Field-less messages would render empty <message> blocks which
        # QuickFIX doesn't love.
        for m in (*SESSION_MESSAGES, *APP_MESSAGES.values()):
            self.assertTrue(m.fields, f"{m.name} has no fields")


if __name__ == "__main__":
    unittest.main()
