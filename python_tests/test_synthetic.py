import zzignal


def test_hello_world():
    msg = zzignal.hello_world()
    assert "ZZignal" in msg


def test_dummy_message():
    out = zzignal.synthetic.dummy_message("David")
    assert "David" in out
    assert "Synthetic" in out


def test_generate_data():
    data = zzignal.synthetic.generate_spy_intraday(
        n_minutes=5,
        s0=100.0,
        drift_ann=0.05,
        vol_ann=0.2,
        seed=42
    )
    assert isinstance(data, list)
    assert len(data) == 5
    assert all(isinstance(x, float) for x in data)
