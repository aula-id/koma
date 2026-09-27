# Embedded pytest plugin: emits bounded structured records without a project file.
import json
import sys
import pytest

def emit(value):
    sys.__stdout__.write('\nKOMA_TEST ' + json.dumps(value, ensure_ascii=True) + '\n')
    sys.__stdout__.flush()

class KomaReporter:
    def pytest_collection_finish(self, session):
        for item in session.items:
            emit({'id': item.nodeid, 'label': item.name, 'file': item.location[0],
                  'line': item.location[1] + 1, 'status': 'discovered'})

    def pytest_runtest_logstart(self, nodeid, location):
        emit({'id': nodeid, 'status': 'running'})

    def pytest_runtest_logreport(self, report):
        if report.when == 'call' or report.failed or report.skipped:
            emit({'id': report.nodeid, 'status': 'failed' if report.failed else 'skipped' if report.skipped else 'passed',
                  'durationMs': report.duration * 1000,
                  'message': str(report.longrepr)[:4096] if report.failed else ''})

sys.exit(pytest.main(sys.argv[1:], plugins=[KomaReporter()]))
