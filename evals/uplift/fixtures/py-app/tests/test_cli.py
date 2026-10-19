from app.cli import main


def test_usage_without_a_command(capsys):
    assert main([]) == 2
    assert "usage" in capsys.readouterr().out
