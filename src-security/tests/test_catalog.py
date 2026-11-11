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


def test_load_packs_skips_debian_and_keeps_cloud_image():
    packs = catalog._load_packs()
    assert packs
    assert all(
        str(row.get("image_or_pack") or "").lower() != "debian" for row in packs.values()
    )
    cloud = packs.get("cloud_enum")
    assert cloud is not None
    assert cloud["image_or_pack"] in {"cloud", "cloud-src"}


def test_index_op_is_compact_and_has_no_debian():
    got = catalog.handle("index")
    tools = got["tools"]
    assert got["total"] == len(tools)
    assert got["total"] > 100
    assert tools
    # `ready` joins once the active guest has been probed.
    assert set(tools[0]) - {"ready"} == {"id", "blurb", "image", "exec", "domain"}
    assert all("how_to" not in row for row in tools)
    assert all(str(row.get("image") or "").lower() != "debian" for row in tools)
    cloud = next(row for row in tools if row["id"] == "cloud_enum")
    assert cloud["image"] in {"cloud", "cloud-src"}


def test_handle_rejects_unknown_op():
    try:
        catalog.handle("nope")
        assert False, "expected ValueError"
    except ValueError:
        pass
