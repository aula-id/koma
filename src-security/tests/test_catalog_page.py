
from koma_sec_daemon import catalog


def test_list_is_paged_by_default():
    page = catalog.handle("list", {"filter": "all"})
    assert "tools" in page
    assert "total" in page
    assert page["limit"] == 40
    assert len(page["tools"]) <= 40
    if page["total"] > 40:
        assert page["next_cursor"] == 40
    else:
        assert page["next_cursor"] is None


def test_list_cursor_advances():
    a = catalog.handle("list", {"limit": 2, "cursor": 0, "filter": "all"})
    b = catalog.handle("list", {"limit": 2, "cursor": 2, "filter": "all"})
    assert len(a["tools"]) <= 2
    if a["total"] > 2:
        assert a["tools"][0]["name"] != b["tools"][0]["name"] or a["total"] <= 2


def test_list_q_filters():
    page = catalog.handle("list", {"q": "http", "limit": 50, "filter": "all"})
    for t in page["tools"]:
        blob = (t.get("name") or "") + (t.get("blurb") or "") + " ".join(t.get("categories") or [])
        assert "http" in blob.lower()


def test_runtime_net_ops_exist():
    from koma_sec_daemon import runtime_ops
    assert callable(runtime_ops.set_net)
    assert callable(runtime_ops.rules_snapshot)
