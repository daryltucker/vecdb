import os
from typing import List, Dict

class DataProcessor:
    """Handles data processing."""
    
    def __init__(self, data: List[str]):
        self.data = data

    async def process(self) -> Dict[str, int]:
        """Process the data asynchronously."""
        result = {}
        for item in self.data:
            result[item] = len(item)
        return result

def main():
    processor = DataProcessor(["item1", "item2"])
    # This is a comment
    print(f"Processing {len(processor.data)} items")
