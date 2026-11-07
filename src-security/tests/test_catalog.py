from koma_sec_daemon import catalog


def test_categories_are_sorted_domains():
    cats = catalog.categories()
    assert cats == sorted(cats)
    assert "web" in cats or "crypto" in cats


def test_list_and_get_round_trip():
    tools = catalog.list_tools()
    assert tools
    name = tools[0]["name"]
    got = catalog.get_tool(name)
    assert got is not None
    assert got["name"] == name
    assert "parameters" in got


def test_handle_rejects_unknown_op():
    try:
        catalog.handle("nope")
        assert False, "expected ValueError"
    except ValueError:
        pass
