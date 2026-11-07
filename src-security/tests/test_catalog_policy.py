from koma_sec_daemon import catalog


def test_smoke_pack_loads():
    packs = catalog.handle("packs")
    assert "smoke" in packs["packs"]


def test_research_hides_exploit_tools():
    tools = catalog.list_tools(lane="research")
    names = {t["name"] for t in tools}
    assert "sec_hashid" in names or "sec_decode" in names
    assert "sec_sqlmap" not in names
    assert "sec_nuclei" not in names


def test_red_sees_web_scanners():
    tools = catalog.list_tools(lane="red", category="web")
    names = {t["name"] for t in tools}
    assert "sec_http" in names or "sec_nuclei" in names


def test_lane_allows():
    assert catalog.lane_allows("sec_hashid", "research")
    assert not catalog.lane_allows("sec_sqlmap", "research")
    assert catalog.lane_allows("sec_sqlmap", "red")
