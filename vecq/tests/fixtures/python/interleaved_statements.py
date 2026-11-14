# Module-level work between definitions: each statement is its own element,
# and none of them contains the `def` beside it (the old `<module>` element
# spanned the first statement to the last, swallowing `helper`).
import logging

logging.basicConfig(level="INFO")


def helper(x):
    return x.strip()


helper(" a ")

if __name__ == "__main__":
    print(helper(" b "))
