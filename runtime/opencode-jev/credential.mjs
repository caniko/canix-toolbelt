import { readFileSync } from "node:fs";
process.env.TYPESAFE_API_KEY = readFileSync(`${process.env.CREDENTIALS_DIRECTORY}/typesafe`, "utf8").trim();
if (!process.env.TYPESAFE_API_KEY) throw new Error("Missing TypeSafe credential");
