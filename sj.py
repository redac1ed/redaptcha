from openai import OpenAI

client = OpenAI(base_url="http://52.59.242.80:8000/v1", api_key="Silverpathx")

response = client.chat.completions.create(
    model="gpt-4-0613",
    messages=[
        {
            "role": "user",
            "content": "Hello!"
        }
    ]
)
print(response.choices[0].message.content)