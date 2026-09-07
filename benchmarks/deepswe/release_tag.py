import re


def validated_release_tag(value):
    if not re.fullmatch(
        r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9][A-Za-z0-9.-]*)?", value
    ):
        raise ValueError("must be a pinned semantic release tag such as v0.14.0")
    return value
