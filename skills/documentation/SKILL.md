---
name: How-To Documentation
description: "This skill should be used when creating documentation, writing instructions, capturing reusable procedures, or when the user asks to 'create a how-to' or 'document a procedure'. Provides guidance for creating effective how-to documentation."
---

# Writing How-To Guides

## When to Write a How-To

Create a how-to when you:
- Do something more than once
- Figure out a non-obvious procedure
- Want to standardize a workflow
- Document a fix that might recur

## Structure of a Good How-To

### 1. Clear Title
Start with "How to..." and be specific:
- Good: "How to deploy to production"
- Bad: "Deployment"

### 2. Context
Brief explanation of when to use this guide:
```markdown
Use this guide when you need to release a new version to production.
Prerequisites: All tests passing, version bumped.
```

### 3. Step-by-Step Instructions
Numbered steps, each doing one thing:
```markdown
1. Ensure all tests pass: `just test`
2. Bump the version in Cargo.toml
3. Create a git tag: `git tag v1.2.3`
4. Push with tags: `git push --tags`
5. Wait for CI to publish
```

### 4. Troubleshooting (Optional)
Common problems and solutions:
```markdown
## Troubleshooting

### CI fails on publish
- Check if version already exists on registry
- Verify credentials are not expired
```

## Creating How-Tos

```
create_howto(
  title="How to add a new API endpoint",
  instructions="... markdown content ..."
)
```

## Linking to Tasks

When a task needs to follow a procedure:
```
link_work_to_howto(work_item_id="...", howto_id="...")
```

Now when viewing the task, the guidance is visible.

## Searching How-Tos

Find relevant guides:
```
search_howtos(query="deploy")
list_howtos()  # See all
```

## Maintaining How-Tos

### Update When Procedures Change
```
update_howto(id="...", instructions="... new content ...")
```

### Delete Obsolete Guides
```
delete_howto(id="...")
```

### Review Periodically
- Are the steps still accurate?
- Has the tooling changed?
- Can any steps be simplified?

## Example: Complete How-To

```markdown
# How to Add a New Database Migration

Use this guide when adding a new table or modifying the schema.

## Prerequisites
- Database access configured
- Migration tool installed

## Steps

1. Create a new migration file:
   ```
   sqlx migrate add <migration_name>
   ```

2. Edit the generated file in `migrations/` with your SQL

3. Test locally:
   ```
   sqlx migrate run
   ```

4. Verify the changes work:
   ```
   cargo test
   ```

5. Commit the migration file

## Troubleshooting

### "migration failed: relation already exists"
The migration may have partially run. Check the database state and either:
- Drop the partial changes manually, or
- Adjust the migration to be idempotent (IF NOT EXISTS)

### "cannot find migration"
Ensure the migration file is in the correct directory and has the `.sql` extension.
```

## Tips

1. **Be specific** - Vague instructions lead to mistakes
2. **Include commands** - Copy-pasteable commands save time
3. **Note prerequisites** - What must be true before starting?
4. **Add troubleshooting** - Document problems you encountered
5. **Keep updated** - Outdated how-tos are worse than none
