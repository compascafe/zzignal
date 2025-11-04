import zzignal

def test_hello_world():
    msg = zzignal.hello_world()
    assert "ZZignal" in msg

def test_dummy_message():
    out = zzignal.dummy_message("David")
    assert "David" in out

def test_generate_data():
    data = zzignal.generate_data(5)
    assert len(data) == 5
